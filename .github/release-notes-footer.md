## Install / First run

These ad-hoc signed builds require macOS 14 or later on Apple silicon. They are distributed without Apple notarization.

- **Rshot.app** — Unzip the download and move `Rshot.app` to Applications. If macOS blocks the first launch, run `xattr -dr com.apple.quarantine Rshot.app` in the folder containing the app. If Finder offers the option, you can also Control-click the app and choose **Open**.
- **rshot-cli** — Extract the archive and run `./rshot-cli --version` from its directory. If macOS blocks the binary, run `xattr -d com.apple.quarantine ./rshot-cli`.
- **Permissions** — In **System Settings > Privacy & Security**, grant **Screen Recording** (called **Screen & System Audio Recording** on newer macOS versions) for screenshot capture and **Accessibility** for UI element inspection. Authorize Rshot for the app and the terminal you use to run `rshot-cli` for the CLI. Restart the authorized app after granting access.
