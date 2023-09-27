use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use heck::{ToLowerCamelCase, ToShoutySnakeCase};
use indent_write::indentable::Indentable;
use itertools::Itertools;
use openapiv3 as oapi;

use crate::{
    operation, schema_by_name, schema_ty, simplify_ty, InputApi, Operation, Property, RequestKind,
    ResponseKind, Type, TypeKind,
};

#[salsa::tracked]
pub fn generate_ts(db: &dyn crate::Db, api: InputApi) -> String {
    use std::fmt::Write;

    let mut buf = String::new();

    writeln!(buf, "{}", include_str!("./preamble.ts")).unwrap();

    let operations = api
        .api(db)
        .paths
        .paths
        .iter()
        .flat_map(|(path, item)| match item {
            oapi::ReferenceOr::Reference { reference: _ } => todo!(),
            oapi::ReferenceOr::Item(path_item) => {
                let span = tracing::debug_span!("endpoint", path);
                let _enter = span.enter();

                if !path_item.parameters.is_empty() {
                    todo!()
                }

                let gen_op = |method: &'static str, op: &Option<oapi::Operation>| {
                    op.as_ref()
                        .map(|op| (method, operation(db, api, path.clone(), op)))
                };
                [
                    gen_op("DELETE", &path_item.delete),
                    gen_op("GET", &path_item.get),
                    gen_op("PUT", &path_item.put),
                    gen_op("POST", &path_item.post),
                    gen_op("HEAD", &path_item.head),
                    gen_op("TRACE", &path_item.trace),
                    gen_op("PATCH", &path_item.patch),
                ]
                .into_iter()
                .flatten()
                .map(|(method, op)| op.ts(db, api, method))
            }
        })
        .collect_vec();

    writeln!(
        buf,
        "export const api = {{\n{}\n}};",
        operations
            .iter()
            .map(|(name, fn_impl)| format!("{name}: {fn_impl},"))
            .format("\n")
            .indented("  ")
    )
    .unwrap();

    tracing::info!("wrote {} operation", operations.len());

    writeln!(buf).unwrap();

    let types = api
        .api(db)
        .components
        .as_ref()
        .unwrap()
        .schemas
        .keys()
        .filter_map(|name| {
            if name.contains('_') {
                tracing::info!(?name, "skipping due to '_'");
                return None;
            }

            let schema = schema_by_name(db, api, name.to_string())?;

            let ty = simplify_ty(db, schema_ty(db, api, schema));

            Some((name, ty))
        })
        .collect_vec();

    for (name, ty) in &types {
        let ts = ty.ts(db);
        writeln!(buf, "export type {name} = {ts};").unwrap();
        if let Some(constants) = ty.constants(db) {
            let const_name =
                pluralizer::pluralize(name, constants.len() as _, false).to_shouty_snake_case();
            writeln!(
                buf,
                "export const {const_name} = [{:?}] satisfies {name}[];",
                constants.iter().format(", ")
            )
            .unwrap();
        }
    }

    tracing::info!("wrote {} types", types.len());

    buf
}

impl Type {
    pub fn ts(self, db: &dyn crate::Db) -> String {
        match self.kind(db) {
            TypeKind::Reference(name) => name.to_string(),
            TypeKind::Object(obj) => {
                if obj.is_empty() {
                    return "{}".to_string();
                }

                let fields = obj
                    .iter()
                    .map(|(name, prop)| {
                        format!(
                            "{name}{}: {};",
                            if prop.optional { "?" } else { "" },
                            prop.ty.ts(db)
                        )
                    })
                    .format("\n")
                    .indented("  ");
                format!("{{\n{fields}\n}}")
            }
            TypeKind::Array(array_ty) => format!("{}[]", array_ty.ts(db)),
            TypeKind::Tuple(elements) => {
                format!("[{}]", elements.iter().map(|ty| ty.ts(db)).join(", "))
            }
            TypeKind::Or(options) => options.iter().map(|opt| opt.ts(db)).join(" | "),
            TypeKind::And(options) => options.iter().map(|opt| opt.ts(db)).join(" & "),
            TypeKind::Number => "number".to_string(),
            TypeKind::String => "string".to_string(),
            TypeKind::Boolean => "boolean".to_string(),
            TypeKind::Ident(ident) => format!("{ident:?}"),
        }
    }
}

impl Operation {
    #[tracing::instrument(skip_all)]
    fn ts(&self, db: &dyn crate::Db, api: InputApi, method: &str) -> (String, String) {
        let path = Utf8PathBuf::from(&self.path);
        let path = if let Some(prefix) = api.config(db).api_prefix {
            path.strip_prefix(prefix).unwrap().to_owned()
        } else {
            path
        };
        let name = path.components().join("_").to_lower_camel_case();

        fn typify_map(db: &dyn crate::Db, map: &BTreeMap<String, Type>) -> Option<Type> {
            if map.is_empty() {
                None
            } else {
                Some(Type::new(
                    db,
                    TypeKind::Object(
                        map.iter()
                            .map(|(name, &ty)| {
                                (
                                    name.clone(),
                                    Property {
                                        ty,
                                        optional: false,
                                    },
                                )
                            })
                            .collect(),
                    ),
                ))
            }
        }

        let params = typify_map(db, &self.path_params);
        let query = typify_map(db, &self.query);
        let json_body = self.body.map(|body| match body {
            RequestKind::Json(body) => body,
        });

        let props = [
            ("params", params),
            ("query", query),
            ("body", json_body),
            (
                "options?",
                Some(Type::new(db, TypeKind::Reference("ApiOptions".to_string()))),
            ),
        ]
        .into_iter()
        .filter_map(|(name, ty)| Some((name, ty?)))
        .collect_vec();

        let url = if params.is_some() {
            format!("`{path}?${{new URLSearchParams(params)}}`")
        } else if query.is_some() {
            format!("`{path}?${{new URLSearchParams(query)}}`")
        } else {
            format!("`{path}`")
        };

        let args = [
            Some(format!("{method:?}")),
            Some(url),
            json_body.map(|_| "body".to_string()),
            Some("options".to_string()),
        ]
        .into_iter()
        .flatten()
        .format(", ");

        let request_impl = match &self.response {
            Some(res) => match res {
                ResponseKind::Plain => {
                    format!("requestPlain({args})",)
                }
                ResponseKind::Json(ty) => {
                    format!("requestJson<{}>({args})", ty.ts(db))
                }
                ResponseKind::EventStream(ty) => {
                    format!("sse<{}>({args})", ty.ts(db))
                }
            },
            None => todo!(),
        };

        (
            name,
            format!(
                "({}) => {request_impl}",
                props
                    .iter()
                    .map(|(name, ty)| format!("{name}: {}", ty.ts(db)))
                    .format(", ")
            ),
        )
    }
}
