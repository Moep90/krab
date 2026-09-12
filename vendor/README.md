# Vendored dependencies

## saphyr-parser 0.0.12

Copied from crates.io with one change in `src/scanner.rs`
(`Scanner::fetch_next_token`): the "invalid indentation" check is skipped while
inside a flow collection. PyYAML accepts

```yaml
key: [
  "a",
  "b"
]
```

and real inventories rely on it; YAML 1.2 (and upstream saphyr) reject the
closing bracket. Everything else is untouched.

Second change, `Scanner::scan_block_scalar`: no newline is synthesised for a
block scalar (`|`) that ends at end-of-stream without a final line break.
PyYAML yields `"x"` for a file ending in `key: |\n  x` (no newline); upstream
saphyr yields `"x\n"`.
