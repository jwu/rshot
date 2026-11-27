# CLI output contract

`rshot-cli` runs on macOS 14 and later. Pass the global `--json` flag before or after a subcommand to receive one JSON object on stdout. By default, successful commands print one absolute artifact path per line, or a short message when they produce no artifacts. Diagnostics and warnings go to stderr.

## Success

JSON responses include the envelope fields and any command-specific fields at the top level.

```json
{"schema_version":1,"ok":true,"command":"capture","image":{"path":"/tmp/capture.png","width":1440,"height":900},"warnings":[]}
```

`warnings` is an array of strings. The names `schema_version`, `ok`, `command`, `warnings`, and `error` are reserved for the envelope. Help and version requests use a `help` or `version` field in JSON mode.

## Failure

Failures exit with a nonzero code. In JSON mode, stdout contains the error response. In text mode, stderr contains the message and any hint. `error.kind` uses `snake_case`, and `error.hint` is a string or `null`. The `command` field uses the subcommand name when available and `rshot-cli` for root-level responses.

```json
{"schema_version":1,"ok":false,"command":"capture","error":{"kind":"permission_denied","message":"Screen recording permission is required.","hint":"Grant screen recording permission in System Settings, then restart Rshot."}}
```

| Exit code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | I/O or save failure |
| `2` | Usage or specification error |
| `3` | Rendering failure |
| `4` | Capture, permission, or dependency failure |
| `5` | Ambiguous target |
| `6` | Target not found |

## Coordinates and artifacts

Image space uses integer physical pixels and a top-left origin. Rectangles use `{"x":0,"y":0,"w":100,"h":80}`. Display space uses points and includes a `scale_factor` giving physical pixels per point.

Every image artifact includes `path`, `width`, and `height`. Dimensions are in physical pixels. Artifact paths are absolute, valid UTF-8, and contain no line breaks.
