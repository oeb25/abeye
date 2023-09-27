mod db;
mod ts;

use camino::Utf8PathBuf;
pub use db::Database;
pub use ts::generate_ts;

use std::collections::BTreeMap;

use itertools::Itertools;
use openapiv3 as oapi;

#[salsa::jar(db = Db)]
pub struct Jar(
    InputApi,
    Type,
    Schema,
    generate_ts,
    schema_by_name,
    schema_ty,
    simplify_ty,
);

pub trait Db: salsa::DbWithJar<Jar> {}

impl<DB> Db for DB where DB: ?Sized + salsa::DbWithJar<Jar> {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub api_prefix: Option<Utf8PathBuf>,
}

#[salsa::input]
pub struct InputApi {
    #[return_ref]
    pub api: oapi::OpenAPI,
    pub config: Config,
}

#[salsa::interned]
struct Type {
    kind: TypeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TypeKind {
    Reference(String),
    Object(BTreeMap<String, Property>),
    Array(Type),
    Tuple(Vec<Type>),
    Or(Vec<Type>),
    And(Vec<Type>),
    Number,
    Ident(String),
    String,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Property {
    ty: Type,
    optional: bool,
}
impl Property {
    fn required(ty: Type) -> Self {
        Property {
            ty,
            optional: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Json(Type),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    Plain,
    Json(Type),
    EventStream(Type),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Operation {
    path: String,
    query: BTreeMap<String, Type>,
    path_params: BTreeMap<String, Type>,
    body: Option<RequestKind>,
    response: Option<ResponseKind>,
}

#[salsa::tracked]
struct Schema {
    #[return_ref]
    schema: OapiSchema,
}

#[derive(Debug, Clone, PartialEq)]
struct OapiSchema {
    schema: oapi::Schema,
}

impl Eq for OapiSchema {}

impl Schema {
    fn from_oapi(db: &dyn crate::Db, schema: oapi::Schema) -> Schema {
        Schema::new(db, OapiSchema { schema })
    }
    fn kind(self, db: &dyn crate::Db) -> &oapi::SchemaKind {
        &self.schema(db).schema.schema_kind
    }
    fn data(self, db: &dyn crate::Db) -> &oapi::SchemaData {
        &self.schema(db).schema.schema_data
    }
}

impl Type {
    pub fn constants(self, db: &dyn crate::Db) -> Option<Vec<String>> {
        match self.kind(db) {
            TypeKind::Or(options)
                if options
                    .iter()
                    .all(|opt| matches!(opt.kind(db), TypeKind::Ident(_))) =>
            {
                Some(
                    options
                        .iter()
                        .map(|opt| match opt.kind(db) {
                            TypeKind::Ident(value) => value.clone(),
                            _ => unreachable!(),
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

fn resolve_schema(
    db: &dyn crate::Db,
    api: InputApi,
    schema: &oapi::ReferenceOr<oapi::Schema>,
) -> Schema {
    match schema {
        oapi::ReferenceOr::Reference { reference } => {
            schema_by_name(db, api, reference.clone()).unwrap()
        }
        oapi::ReferenceOr::Item(schema) => Schema::from_oapi(db, schema.clone()),
    }
}
fn resolve_schema_ty(
    db: &dyn crate::Db,
    api: InputApi,
    schema: &oapi::ReferenceOr<oapi::Schema>,
) -> Type {
    schema_ty(db, api, resolve_schema(db, api, schema))
}
fn shallow_schema_ty(
    db: &dyn crate::Db,
    api: InputApi,
    schema: &oapi::ReferenceOr<oapi::Schema>,
) -> Type {
    match schema {
        oapi::ReferenceOr::Reference { reference } => {
            if let Some(name) = reference.strip_prefix("#/components/schemas/") {
                if name.contains('_') {
                    resolve_schema_ty(db, api, schema)
                } else {
                    Type::new(db, TypeKind::Reference(name.to_string()))
                }
            } else {
                todo!()
            }
        }
        oapi::ReferenceOr::Item(schema) => {
            schema_ty(db, api, Schema::from_oapi(db, schema.clone()))
        }
    }
}

fn ty_by_name(db: &dyn crate::Db, api: InputApi, name: String) -> Type {
    shallow_schema_ty(db, api, &oapi::ReferenceOr::Reference { reference: name })
}

fn operation(
    db: &dyn crate::Db,
    api: InputApi,
    path: String,
    operation: &oapi::Operation,
) -> Operation {
    let mut path_params = BTreeMap::new();
    let mut query = BTreeMap::new();

    for param in &operation.parameters {
        match param {
            oapi::ReferenceOr::Reference { .. } => todo!(),
            oapi::ReferenceOr::Item(param) => match param {
                oapi::Parameter::Query { parameter_data, .. } => {
                    let ty = match &parameter_data.format {
                        oapi::ParameterSchemaOrContent::Schema(schema) => {
                            shallow_schema_ty(db, api, schema)
                        }
                        oapi::ParameterSchemaOrContent::Content(_) => todo!(),
                    };

                    query.insert(parameter_data.name.clone(), ty);
                }
                oapi::Parameter::Header { .. } => todo!(),
                oapi::Parameter::Path { parameter_data, .. } => {
                    let ty = match &parameter_data.format {
                        oapi::ParameterSchemaOrContent::Schema(schema) => {
                            shallow_schema_ty(db, api, schema)
                        }
                        oapi::ParameterSchemaOrContent::Content(_) => todo!(),
                    };

                    path_params.insert(parameter_data.name.clone(), ty);
                }
                oapi::Parameter::Cookie { .. } => todo!(),
            },
        }
    }
    let body = if let Some(body) = &operation.request_body {
        match body {
            oapi::ReferenceOr::Reference { .. } => todo!(),
            oapi::ReferenceOr::Item(body) => {
                assert_eq!(body.content.len(), 1);

                let (media_type, value) = body.content.iter().next().unwrap();
                let ty = if let Some(schema) = &value.schema {
                    let ty = simplify_ty(db, shallow_schema_ty(db, api, schema));
                    let ts = ty.ts(db);
                    tracing::debug!(?media_type, ty=?ts, "request");
                    ty
                } else {
                    todo!()
                };
                match media_type.as_str() {
                    "application/json" => Some(RequestKind::Json(ty)),
                    _ => todo!("unhandled request media type: {media_type:?}"),
                }
            }
        }
    } else {
        None
    };

    if !path_params.is_empty() {
        for (path_param, ty) in &path_params {
            let ty = ty.ts(db);
            tracing::debug!(?path_param, ?ty);
        }
    }
    if !query.is_empty() {
        for (query_param, ty) in &query {
            let ty = ty.ts(db);
            tracing::debug!(?query_param, ?ty);
        }
    }

    let mut response = None;

    for (status, res) in &operation.responses.responses {
        response = match res {
            oapi::ReferenceOr::Reference { .. } => todo!(),
            oapi::ReferenceOr::Item(response) => {
                for (media_type, value) in &response.content {
                    if let Some(schema) = &value.schema {
                        let ty = simplify_ty(db, shallow_schema_ty(db, api, schema)).ts(db);
                        tracing::debug!(?status, ?media_type, ?ty, "response");
                    }
                }

                assert_eq!(response.content.len(), 1);

                let (media_type, value) = response.content.iter().next().unwrap();
                let ty = if let Some(schema) = &value.schema {
                    let ty = simplify_ty(db, shallow_schema_ty(db, api, schema));
                    let ts = ty.ts(db);
                    tracing::debug!(?status, ?media_type, ty=?ts, "response");
                    ty
                } else {
                    todo!()
                };
                match media_type.as_str() {
                    "text/plain" => {
                        assert_eq!(ty, Type::new(db, TypeKind::String));
                        Some(ResponseKind::Plain)
                    }
                    "application/json" => Some(ResponseKind::Json(ty)),
                    "text/event-stream" => Some(ResponseKind::EventStream(ty)),
                    _ => todo!("unhandled request media type: {media_type:?}"),
                }
            }
        };
    }

    Operation {
        path,
        query,
        path_params,
        body,
        response,
    }
}

#[salsa::tracked]
fn schema_by_name(db: &dyn crate::Db, api: InputApi, name: String) -> Option<Schema> {
    tracing::debug!(?name, "schema_by_name");

    if let Some(name) = name.strip_prefix("#/components/schemas/") {
        schema_by_name(db, api, name.to_string())
    } else {
        match api.api(db).components.as_ref()?.schemas.get(&name)? {
            oapi::ReferenceOr::Reference { reference } => {
                todo!("reference to: {reference}")
            }
            oapi::ReferenceOr::Item(schema) => Some(Schema::from_oapi(db, schema.clone())),
        }
    }
}

#[salsa::tracked]
fn schema_ty(db: &dyn crate::Db, api: InputApi, schema: Schema) -> Type {
    match schema.kind(db) {
        oapi::SchemaKind::Type(ty) => match ty {
            oapi::Type::String(str) => {
                if str.enumeration.is_empty() {
                    Type::new(db, TypeKind::String)
                } else {
                    Type::new(
                        db,
                        TypeKind::Or(
                            str.enumeration
                                .iter()
                                .map(|e| Type::new(db, TypeKind::Ident(e.clone().unwrap())))
                                .collect(),
                        ),
                    )
                }
            }

            oapi::Type::Number(_) | oapi::Type::Integer(_) => Type::new(db, TypeKind::Number),
            oapi::Type::Object(obj) => {
                let mut properties = BTreeMap::default();

                for (name, prop) in &obj.properties {
                    let ty = shallow_schema_ty(db, api, &prop.clone().unbox());
                    let required = obj.required.contains(name);
                    properties.insert(
                        name.clone(),
                        Property {
                            ty,
                            optional: !required,
                        },
                    );
                }

                if let Some(disc) = &schema.data(db).discriminator {
                    assert!(disc.extensions.is_empty());

                    match disc.mapping.len() {
                        0 => todo!(),
                        1 => todo!(),
                        _ => Type::new(
                            db,
                            TypeKind::Or(
                                disc.mapping
                                    .iter()
                                    .map(|(name, rest)| {
                                        let ty = ty_by_name(db, api, rest.clone());
                                        let marker = Type::new(
                                            db,
                                            TypeKind::Object(
                                                [(
                                                    disc.property_name.clone(),
                                                    Property::required(Type::new(
                                                        db,
                                                        TypeKind::Ident(name.clone()),
                                                    )),
                                                )]
                                                .into_iter()
                                                .collect(),
                                            ),
                                        );
                                        Type::new(db, TypeKind::And(vec![marker, ty]))
                                    })
                                    .collect(),
                            ),
                        ),
                    }
                } else {
                    Type::new(db, TypeKind::Object(properties))
                }
            }
            oapi::Type::Array(array_ty) => {
                let ty = shallow_schema_ty(db, api, &array_ty.items.clone().unwrap().unbox());
                match (array_ty.min_items, array_ty.max_items) {
                    (Some(min), Some(max)) if min == max => {
                        Type::new(db, TypeKind::Tuple(vec![ty; min]))
                    }
                    (None, None) => Type::new(db, TypeKind::Array(ty)),
                    (min, max) => todo!("{:?}", (min, max)),
                }
            }
            oapi::Type::Boolean {} => Type::new(db, TypeKind::Boolean),
        },
        oapi::SchemaKind::OneOf { one_of } => Type::new(
            db,
            TypeKind::Or(
                one_of
                    .iter()
                    .map(|item| shallow_schema_ty(db, api, item))
                    .collect(),
            ),
        ),
        oapi::SchemaKind::AllOf { all_of } => Type::new(
            db,
            TypeKind::And(
                all_of
                    .iter()
                    .map(|item| shallow_schema_ty(db, api, item))
                    .collect(),
            ),
        ),
        oapi::SchemaKind::AnyOf { .. } => todo!(),
        oapi::SchemaKind::Not { .. } => todo!(),
        oapi::SchemaKind::Any(_) => todo!(),
    }
}

#[salsa::tracked]
fn simplify_ty(db: &dyn crate::Db, ty: Type) -> Type {
    match ty.kind(db) {
        TypeKind::Reference(_) => ty,
        TypeKind::Object(obj) => Type::new(
            db,
            TypeKind::Object(
                obj.iter()
                    .map(|(name, prop)| {
                        (
                            name.clone(),
                            Property {
                                ty: simplify_ty(db, prop.ty),
                                optional: prop.optional,
                            },
                        )
                    })
                    .collect(),
            ),
        ),
        TypeKind::Array(array_ty) => Type::new(db, TypeKind::Array(simplify_ty(db, array_ty))),
        TypeKind::Tuple(elements) => Type::new(
            db,
            TypeKind::Tuple(elements.iter().map(|ty| simplify_ty(db, *ty)).collect()),
        ),
        TypeKind::Or(options) => Type::new(
            db,
            TypeKind::Or(
                options
                    .iter()
                    .map(|opt| simplify_ty(db, *opt))
                    .sorted()
                    .dedup()
                    .collect(),
            ),
        ),
        TypeKind::And(options) => {
            let options = options
                .iter()
                .map(|opt| simplify_ty(db, *opt))
                .sorted()
                .dedup()
                .collect_vec();

            if options
                .iter()
                .all(|opt| matches!(opt.kind(db), TypeKind::Object(_)))
            {
                let mut fields = BTreeMap::new();

                for opt in options {
                    match opt.kind(db) {
                        TypeKind::Object(fs) => {
                            for (field, ty) in fs {
                                let Some(old) = fields.insert(field.clone(), ty) else {
                                    continue;
                                };
                                if let (TypeKind::Ident(_), TypeKind::String) =
                                    (old.ty.kind(db), ty.ty.kind(db))
                                {
                                    fields.insert(field, old);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }

                Type::new(db, TypeKind::Object(fields))
            } else {
                Type::new(db, TypeKind::And(options))
            }
        }
        TypeKind::Number | TypeKind::String | TypeKind::Boolean | TypeKind::Ident(_) => ty,
    }
}
