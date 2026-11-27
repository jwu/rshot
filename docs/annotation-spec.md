# Annotation spec v1

`rshot-cli annotate INPUT --spec PATH [--out PATH]` applies a UTF-8 JSON spec to a PNG. Use `--spec -` to read stdin or pass a quoted JSON object beginning with `{` as the `--spec` value. The default output is `<input stem>.annotated.png` beside the input.

Set `schema_version` to `1` and supply a `marks` array. Marks are drawn in array order, with later marks appearing on top. Coordinates are integer physical pixels from the image's upper-left corner. Points use `{"x":0,"y":0}` and rectangles use `{"x":0,"y":0,"w":100,"h":80}`. Rectangle dimensions must be nonnegative. Off-image marks are clipped in place and reported as warnings.

| `type` | Required fields | Optional fields and defaults |
| --- | --- | --- |
| `rect` | `rect` for the outer bounds | `stroke` `"#E63946"`, `width` `3`, `radius` `0`, `fill` `null` |
| `ellipse` | `rect` for the outer bounds | `stroke` `"#276EF1"`, `width` `3`, `fill` `null` |
| `arrow` | `from` at the tail, `to` at the tip | `stroke` `"#E63946"`, `width` `4` |
| `text` | `at` at the upper-left of the text box | `text` `""`, `size` `22`, `color` `"#111111"`, `background` `null` |
| `step` | `at` at the badge's center | `n` `null`, `size` `28`, `fill` `"#276EF1"`, `color` `"#FFFFFF"`, `label` `null` |
| `highlight` | `rect` for the area | `color` `"#FFD84E"`, `opacity` `0.4`, `blend` `"multiply"` |
| `redact` | `rect` for the area | `fill` `"#000000"` |

`width` and `size` are positive integers in pixels. The rect `radius` is a nonnegative integer. Rect and ellipse strokes sit inside their outer bounds. Colors accept `#RGB`, `#RRGGBB`, and `#RRGGBBAA`. Redact replaces all four channels of the covered pixels with `fill`.

Highlight `opacity` ranges from 0 to 1 and multiplies the color's alpha. The default `"multiply"` blend tints light areas and keeps dark text dark. Set `blend` to `"normal"` for a source-over color overlay.

A step's `size` is its diameter. An omitted or `null` `n` starts at `1` or continues from one more than the highest preceding step number. Explicit numbers are positive integers. The badge shows its number, and its optional `label` is returned in the legend. Use a `text` mark for a visible label.

## Example

Save this spec as `steps.json` and run `rshot-cli annotate settings.png --spec steps.json --json`. The two badges are numbered `1` and `2`.

```json
{
  "schema_version": 1,
  "marks": [
    {"type":"redact","rect":{"x":611,"y":264,"w":186,"h":20}},
    {"type":"rect","rect":{"x":262,"y":363,"w":690,"h":53},"radius":8,"width":3},
    {"type":"ellipse","rect":{"x":873,"y":372,"w":58,"h":36},"width":2},
    {"type":"arrow","from":{"x":837,"y":460},"to":{"x":874,"y":413},"width":3},
    {"type":"text","at":{"x":650,"y":451},"text":"Enable weekly digest","size":16,"background":"#FFFFFFE6"},
    {"type":"highlight","rect":{"x":813,"y":549,"w":132,"h":54},"opacity":0.22},
    {"type":"step","at":{"x":240,"y":390},"n":null,"label":"Enable the weekly digest"},
    {"type":"step","at":{"x":969,"y":575},"label":"Save changes"}
  ]
}
```

The JSON response includes `image` and `source` with their absolute paths and dimensions, `legend` in step order, and one `marks` entry per input mark. Each entry gives a zero-based `index`, `type`, an `n` for steps, and pixel `bounds` after clipping. Marks that cover no pixels have zero-sized bounds. See the [CLI output contract](cli-contract.md) for the shared response envelope and exit codes.
