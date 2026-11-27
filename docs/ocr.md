# OCR and text location

On macOS 14 and later, `rshot-cli ocr` and `rshot-cli locate` use Apple Vision to recognize text locally in a PNG. Rectangles use physical pixels from the image's upper-left corner.

```sh
rshot-cli ocr screenshot.png --json
rshot-cli ocr screenshot.png --lang zh-Hans --lang en --json
rshot-cli ocr screenshot.png --lang en --fast --json
rshot-cli locate screenshot.png 'Save Changes' --json
rshot-cli locate screenshot.png Save --all --json
rshot-cli locate screenshot.png Save --nth 2 --json
```

## Recognize text

`ocr INPUT [--lang L]... [--fast] [--json]` returns a `lines` array with `text`, `rect`, and `confidence` for each line. An image with no detectable text returns an empty array. The default is accurate recognition with automatic language detection. Repeat `--lang` to set the language preference order. The first language can affect which scripts Vision detects in a mixed image. `en` is an alias for `en-US`, and `zh-Hans` selects Simplified Chinese. Language availability depends on the recognition level supported by Vision. An unavailable language returns exit 2 with `unsupported_language`.

## Locate text

`locate INPUT TEXT [--exact] [--case-sensitive] [--nth N | --all] [--json]` uses accurate OCR with automatic language detection. By default it finds case-insensitive substrings within a line. Use `--exact` to match the whole line and `--case-sensitive` to preserve letter case when matching.

Matches are ordered by line position, top to bottom then left to right at the same height, and by occurrence in the text within each line. `--nth` uses a one-based index. `--all` lists every nonoverlapping occurrence. With neither option, the query must resolve to one occurrence.

```json
{"schema_version":1,"ok":true,"command":"locate","matches":[{"text":"Save","rect":{"x":50,"y":44,"w":91,"h":33},"confidence":1.0,"estimated":false}],"warnings":[]}
```

Each match preserves the recognized spelling and line confidence. `estimated` is `false` for a whole-line rectangle or a character-range rectangle returned by Vision. Vision's range boxes can extend beyond individual glyphs. If Vision cannot resolve the matched range, such as when it returns the entire word for a partial word, Rshot estimates the horizontal bounds from the matching characters' proportion of the line and sets `estimated` to `true`.

| Exit code | Error kind | Response |
| --- | --- | --- |
| `5` | `ambiguous_target` | `error.matches` lists every candidate with the same fields as successful `matches`. |
| `6` | `text_not_found` | `error.hint` lists up to three similar recognized lines when available. This also covers an out-of-range `--nth`. |

Without `--json`, each result occupies one line with tab-separated rectangle (`x,y,w,h`), confidence, and text. `locate` appends the `estimated` flag. See the [CLI output contract](cli-contract.md) for the shared envelope and exit codes.
