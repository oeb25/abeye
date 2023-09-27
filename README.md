# abeye

> 🐝 OpenAPI client generator

## Usage

```bash
❯ abeye --help
🐝 OpenAPI client generator

Usage: abeye <COMMAND>

Commands:
  generate  Generate type definitions and client for the given OpenAPI
  help      Print this message or the help of the given subcommand(s)
```

```bash
❯ abeye generate --help
Generate type definitions and client for the given OpenAPI

Usage: abeye generate [OPTIONS] --target <TARGET> [SOURCE]

Arguments:
  [SOURCE]
          Path or URL of the OpenAPI document. If none is provided the document will be read from STDIN

Options:
  -t, --target <TARGET>
          The output format of the generated file

          [possible values: ts]

  -o, --output <OUTPUT>
          The path where the output will be written. If none is provided the out generated file will be printed to STDOUT

      --api-prefix <API_PREFIX>
          A common prefix for API endpoints to exclude when determining names generated methods.

          For example, given "/beta/api" the endpoints will have names as follows:

          * "/beta/api/autosuggest"            => "autosuggest"

          * "/beta/api/explore/export"         => "exploreExport"

          * "/beta/api/sites/export"           => "sitesExport"

          * "/beta/api/webgraph/host/ingoing"  => "webgraphHostIngoing"

          * "/beta/api/webgraph/host/outgoing" => "webgraphHostOutgoing"
```
