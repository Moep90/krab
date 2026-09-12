# Fixture inventory

A small inventory exercising the features the engine must reproduce exactly:
class resolution (init.yml, relative classes, diamonds), EXTEND_UNIQUE lists,
merge-time dereferencing, every shipped resolver, YAML 1.1 scalars, kapitan
model normalisation, and PyYAML emitter quirks (folding, quoting, unicode).

`expected/<target>.yaml` is what kapitan 0.36.3 (omegaconf backend) prints for
`kapitan inventory -t <target>`. Regenerate with the reference implementation:

```sh
cd tests/fixtures
PEX_INTERPRETER=1 kapitan generate_expected.py
```
