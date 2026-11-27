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

`locate` includes the candidate matches as `error.matches` on an ambiguous query. See [OCR and text location](ocr.md) for its result fields and hints.

## Coordinates and artifacts

Image space uses integer physical pixels and a top-left origin. Rectangles use `{"x":0,"y":0,"w":100,"h":80}`. Display space uses points and includes a `scale_factor` giving physical pixels per point.

Every image artifact includes `path`, `width`, and `height`. Dimensions are in physical pixels. Artifact paths are absolute, valid UTF-8, and contain no line breaks.

## Accessibility elements

Use `rshot-cli elements --window ID [--role ROLE]... [--max N] --json` to list elements in the owning process's accessibility tree whose bounds intersect the specified WindowServer window. Get the ID from `rshot-cli windows`. Roles match exactly, for example `AXButton`, `AXTextField`, `AXLink`, or `AXCheckBox`. The default maximum is 200 and `--max` accepts 1 through 1,000. The search visits up to 5,000 nodes and descends up to 32 levels from the application root. The response includes `window_id`, `window_bounds`, `scale_factor`, `elements`, and `truncated`. A `true` value for `truncated` indicates that a traversal limit was reached or matching elements were omitted. Each `id` is a one-based traversal index. IDs can have gaps and are reproducible across filters for an unchanged accessibility tree.

An element includes `role`, `bounds`, and optional `subrole`, `title`, `description`, `value`, `identifier`, `enabled`, and `focused` fields. Unavailable attributes are `null`. `value` represents supported scalar values as strings and is capped at 256 Unicode characters. The element's `bounds` and `window_bounds` are in global points, with the origin at the top left of the primary display. Displays arranged to the left or above it can have negative coordinates.

Every listed element includes `image_rect` in the local pixel coordinates of a `capture --window ID` image. Intersect the global element bounds with the ScreenCaptureKit window bounds, subtract the window origin from the intersected edges, then multiply by the window filter's `pointPixelScale` exposed as `scale_factor`. Round the leading edges down and the trailing edges up, clamp them to the image while preserving at least one pixel for a nonempty intersection, and take the differences for `w` and `h`. Pass `image_rect` directly as a rectangle in an annotation spec. List elements and capture the window before moving or resizing it. The window query uses Screen Recording permission to obtain the ScreenCaptureKit geometry.

Use `rshot-cli elements --at X,Y --json` to hit-test the topmost accessible UI at a global point. The response contains `element` and up to 16 `ancestors`, ordered from the nearest parent outward. Each ancestor includes a `role` and `bounds`, with `null` for unavailable attributes.

Use `rshot-cli capture --element-at X,Y --json` to take a display-region screenshot of the hit element. For a cross-display element, the screenshot includes the part on the display that contains the hit point. The response adds `capture.element` with the hit element's metadata and reports the clipped, pixel-aligned screenshot bounds in `capture.bounds`. `--element-at` is exclusive with `--display`, `--window`, and `--region`.

Grant Accessibility permission to your terminal or calling app in System Settings → Privacy & Security → Accessibility. Missing permission returns exit `4` and `accessibility_denied`. AX messaging failures return exit `4` and `accessibility_failed` or `accessibility_timeout`. A locked session returns exit `4` and `screen_locked`. An unknown window or an element without usable bounds returns exit `6` and `window_not_found` or `element_not_found`.
