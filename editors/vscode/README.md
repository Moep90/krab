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

## Build and install

```sh
cd editors/vscode
npm install
npx vsce package          # produces kapitan-0.1.0.vsix
code --install-extension kapitan-0.1.0.vsix
```

Set `kapitan.path` if the binary is not on `PATH` (for example `kapitan2`
while both implementations are installed).
