# Rshot

Rshot is a macOS app for capturing, annotating, and pinning screenshots, with a command-line interface for automation and AI agents. Rshot supports macOS 14 or later on Apple Silicon.

## Install

Download and extract the archive you need from [GitHub Releases](https://github.com/jwu/rshot/releases).

- `Rshot-<ver>-aarch64-apple-darwin.zip` contains `Rshot.app`.
- `rshot-cli-<ver>-aarch64-apple-darwin.tar.gz` contains `rshot-cli`. Move it to a directory on your `PATH`.

Both packages are unnotarized. On first launch, Control-click the extracted app or binary in Finder and choose **Open**. If macOS still blocks it, remove the quarantine attribute from the corresponding extracted item. Run these from the directory containing the extracted files, or adjust the paths.

```sh
xattr -dr com.apple.quarantine Rshot.app
xattr -dr com.apple.quarantine rshot-cli
```

To install from a checkout, use Rust, Cargo, and the macOS Command Line Tools. The GUI executable installed by Cargo is named `rshot`.

```sh
cargo install --locked --path crates/rshot-cli
cargo install --locked --path crates/rshot-gui
```

## Permissions

In **System Settings → Privacy & Security**, grant the permissions required by the operation.

- **Screen Recording** (called **Screen & System Audio Recording** on newer macOS versions) allows screenshots in Rshot and `rshot-cli capture`, and provides the window geometry for `rshot-cli elements --window`.
- **Accessibility** allows `rshot-cli elements` to inspect UI elements and `rshot-cli capture --element-at` to find the element to capture.

For the CLI, grant permissions to the terminal or host application running it, then quit and relaunch that host. For GUI screenshots, grant Screen Recording to Rshot and relaunch it. The offline commands `annotate`, `grid`, `zoom`, `ocr`, and `locate` require neither permission.

## GUI

- Drag to select a rectangle or click the window or macOS menu bar under the pointer. The current display is the fallback when the pointer hits neither.
- Add arrows, text, and mosaic blocks in the screenshot editor. Undo and redo edits with `⌘Z` and `⌘⇧Z`.
- Copy the result to the clipboard, save it as a PNG, or pin a captured selection above other windows. You can also pin an image or plain text from the clipboard.
- Stitch a vertical or horizontal scrolling screenshot. Scroll the source manually and use **Add frame** (`添加一帧`) to capture overlapping frames, then copy or save the result.

## CLI

| Command | What it does |
| --- | --- |
| `windows` | List windows and their IDs. |
| `capture` | Capture a display, window, rectangular region, or accessibility element as PNG. |
| `elements` | List accessibility elements intersecting a window or find the element at a screen point. |
| `locate` | Find the pixel rectangles of matching text in a PNG. |
| `ocr` | Recognize text in a PNG and return line rectangles and confidence scores. |
| `annotate` | Apply a JSON annotation spec to a PNG. |
| `grid` | Draw a coordinate grid over a PNG. |
| `zoom` | Enlarge a pixel region of a PNG and overlay a grid. |

An agent can list windows, capture the chosen window, and use the pixel rectangle returned by `locate` to annotate visible text. Use `elements` when you need an accessibility element's role or its `image_rect` in the same window image. Keep the window in place between capture and inspection. This example uses `jq` to build the annotation spec.

```sh
rshot-cli windows --json
```

Run `windows` without `--app` first to check the actual application name. `--app` matches the localized name, such as `访达` for Finder on a Chinese-language system.

Set `window_id` to a `window_id` from the response and `target_text` to text that appears once in that window.

```sh
window_id=123
target_text='Save Changes'
rshot-cli capture --window "$window_id" --out window.png --json
rshot-cli locate window.png "$target_text" --json > match.json
rshot-cli elements --window "$window_id" --json
jq -e '.matches[0].rect // empty | {schema_version: 1, marks: [{type: "rect", rect: .}]}' match.json > marks.json
rshot-cli annotate window.png --spec marks.json --out annotated.png --json
```

[CLI output contract](docs/cli-contract.md) · [Annotation spec](docs/annotation-spec.md) · [OCR and text location](docs/ocr.md)

## Development

Run `just check` for formatting, lint, and tests. Use `just wt` to manage task worktrees. See the [contribution conventions](AGENTS.md) and [release procedure](docs/releasing.md).

## License

Rshot is licensed under either the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in Rshot, as defined by the Apache License, Version 2.0, is dual-licensed as above, without additional terms or conditions.
