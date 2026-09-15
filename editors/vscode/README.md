# Kapitan for VS Code

Live diagnostics for the inventory (render errors show up as Problems as you
save class and target files), hover on any parameter key or `${…}` reference
to see the resolved value in the targets that include the file and where it
was written or overridden, go to definition on class names and references,
and completion of class names and parameter paths.

The extension launches `kapitan lsp` (the Rust kapitan) in the workspace
folder containing `.kapitan` (or a first-level subdirectory such as `grid/`).
The language server talks to the kapitan inventory daemon, which it starts if
needed, so results are always the daemon's current render.

## Install

Every [krab release](https://github.com/kapicorp/krab/releases) attaches the
packaged extension:

```sh
code --install-extension kapitan-vscode-0.1.0.vsix
```

## Build

```sh
cd editors/vscode
npm install
npm run package           # produces kapitan-vscode-0.1.0.vsix
code --install-extension kapitan-vscode-0.1.0.vsix
```

CI packages the extension on every pull request (the `kapitan-vscode`
workflow artifact) and the release workflow attaches it to the release.

Set `kapitan.path` if the binary is not on `PATH` (for example `kapitan2`
while both implementations are installed).
