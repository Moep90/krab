# Kapitan for VS Code

Live diagnostics for the inventory (render errors show up as Problems as you
save class and target files), hover on any parameter key or `${…}` reference
to see the resolved value in the targets that include the file and where it
was written or overridden, go to definition on class names and references,
and completion of class names and parameter paths.

The extension launches `krab lsp` (the Rust kapitan) in the workspace
folder containing `.kapitan` (or a first-level subdirectory such as `grid/`).
The language server talks to the krab inventory daemon, which it starts if
needed, so results are always the daemon's current render.

## Install

Every [krab release](https://github.com/kapicorp/krab/releases) attaches the
packaged extension:

```sh
code --install-extension krab-vscode-0.1.0.vsix
```

## Build

```sh
cd editors/vscode
npm install
npm run package           # produces krab-vscode-0.1.0.vsix
code --install-extension krab-vscode-0.1.0.vsix
```

CI packages the extension on every pull request (the `krab-vscode`
workflow artifact) and the release workflow attaches it to the release.

Set `krab.path` if `krab` is not on the extension host's `PATH`, or to
use a development build. Point it at the same binary your shell uses: each
build keeps its own inventory daemon, so two builds mean two daemons
rendering the same inventory.

If the inventory has Python resolvers, set `krab.python` to an interpreter
that can import omegaconf (a venv or pixi environment python). The language
server inherits the editor's environment, whose `python3` usually cannot, and
the daemon it starts would fail to load `resolvers.py`.
