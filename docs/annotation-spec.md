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
| `mosaic` | `rect` for the area | `block` `12` |
| `blur` | `rect` for the area | `radius` `12` |
| `spotlight` | `rects` with one or more focus rectangles | `shape` `"rect"`, `opacity` `0.6`, `color` `"#000000"` |
| `magnify` | `source` for the area to sample, `at` at the upper-left of the inset | `scale` `2.0`, `border` `"#276EF1"`, `width` `3`, `connector` `true` |

`width` and `size` are positive integers in pixels. The rect `radius` is a nonnegative integer. Rect and ellipse strokes sit inside their outer bounds. Colors accept `#RGB`, `#RRGGBB`, and `#RRGGBBAA`. Redact replaces all four channels of the covered pixels with `fill`.

Highlight `opacity` ranges from 0 to 1 and multiplies the color's alpha. The default `"multiply"` blend tints light areas and keeps dark text dark. Set `blend` to `"normal"` for a source-over color overlay.

A step's `size` is its diameter. An omitted or `null` `n` starts at `1` or continues from one more than the highest preceding step number. Explicit numbers are positive integers. The badge shows its number, and its optional `label` is returned in the legend. Use a `text` mark for a visible label.

Mosaic averages the RGB channels in square blocks within the visible part of `rect` and makes them opaque. Blur applies a Gaussian approximation to the visible part of `rect` using `radius` as its standard deviation in pixels. The effective radius is capped at the longest visible side. `block` and `radius` are positive integers. Values below `8` produce a warning and still render. Mosaic and blur are unreliable for hiding sensitive information at any setting. Use an opaque `redact` mark to cover sensitive pixels.

Spotlight applies the overlay outside the union of `rects`. Use `"rect"` for rectangular focus areas or `"ellipse"` for ellipses inscribed in those rectangles. `opacity` ranges from 0 to 1 and scales the overlay color's alpha.

Magnify clips `source` to the input image, rounds its scaled dimensions to the nearest pixel, and samples it using nearest-neighbor interpolation. `scale` must be finite and at least `1`, and each scaled dimension of the requested `source` must fit in an unsigned 32-bit integer. `width` sets the inset's inside border thickness and the connector's stroke width. With `connector: true`, a line runs from the center of the sampled region to the center of the inset and is drawn beneath it. Magnify samples the original input, including pixels covered by earlier marks. Keep sensitive areas out of `source`, and apply opaque `redact` to any inset that contains them.

## Examples

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

An effects spec can combine multiple spotlight focus areas with local filters and an inset from the same image.

```json
{
  "schema_version": 1,
  "marks": [
    {"type":"spotlight","rects":[{"x":262,"y":363,"w":690,"h":53},{"x":813,"y":549,"w":132,"h":54}]},
    {"type":"mosaic","rect":{"x":273,"y":373,"w":126,"h":32},"block":12},
    {"type":"blur","rect":{"x":450,"y":373,"w":137,"h":32},"radius":12},
    {"type":"magnify","source":{"x":850,"y":367,"w":87,"h":43},"at":{"x":620,"y":440},"scale":2.0}
  ]
}
```

The JSON response includes `image` and `source` with their absolute paths and dimensions, `legend` in step order, and one `marks` entry per input mark. Each entry gives a zero-based `index`, `type`, an `n` for steps, and pixel `bounds` after clipping. Marks that cover no pixels have zero-sized bounds. See the [CLI output contract](cli-contract.md) for the shared response envelope and exit codes.
