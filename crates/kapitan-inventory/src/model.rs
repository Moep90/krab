//! Kapitan's typed view of `parameters.kapitan`. The reference implementation
//! validates the rendered parameters against pydantic models, which fills in
//! defaults and rejects unknown fields; this module does the same so that
//! `kapitan inventory` output is identical.

use crate::error::{Error, Result};
use crate::source::Origin;
use crate::value::{Map, Node, Value};

/// The `parameters` every target starts from before any class is merged:
/// default kapitan settings plus the `_kapitan_` / `_reclass_` metadata.
pub fn initial_parameters(name: &str, path_without_ext: &str) -> Node {
    let o = Origin::SYNTHETIC;
    let n = |v: Value| Node::new(v, o);
    let s = |v: &str| n(Value::Str(v.to_string()));
    let kapitan = Map::from_iter([
        ("compile".into(), n(Value::List(vec![]))),
        (
            "vars".into(),
            n(Value::Map(Map::from_iter([(
                "target".into(),
                n(Value::Null),
            )]))),
        ),
        ("labels".into(), n(Value::Map(Map::new()))),
        ("dependencies".into(), n(Value::List(vec![]))),
        ("target_full_path".into(), s("")),
        ("secrets".into(), n(Value::Null)),
        ("validate".into(), n(Value::List(vec![]))),
    ]);
    let meta = || {
        n(Value::Map(Map::from_iter([(
            "name".into(),
            n(Value::Map(Map::from_iter([
                ("short".into(), s(name.rsplit('.').next().unwrap_or(name))),
                ("full".into(), s(name)),
                ("path".into(), s(path_without_ext)),
                (
                    "parts".into(),
                    n(Value::List(name.split('.').map(s).collect())),
                ),
            ]))),
        )])))
    };
    n(Value::Map(Map::from_iter([
        ("kapitan".into(), n(Value::Map(kapitan))),
        ("_kapitan_".into(), meta()),
        ("_reclass_".into(), meta()),
    ])))
}

/// A field of a pydantic model: name and default (`None` = required).
struct Field {
    name: &'static str,
    default: Option<fn() -> Value>,
}

const fn req(name: &'static str) -> Field {
    Field {
        name,
        default: None,
    }
}

const fn opt(name: &'static str, default: fn() -> Value) -> Field {
    Field {
        name,
        default: Some(default),
    }
}

fn null() -> Value {
    Value::Null
}
fn f() -> Value {
    Value::Bool(false)
}
fn t() -> Value {
    Value::Bool(true)
}
fn empty_map() -> Value {
    Value::Map(Map::new())
}
fn empty_list() -> Value {
    Value::List(vec![])
}
fn yaml() -> Value {
    Value::Str("yaml".into())
}

const COMPILE_BASE: &[Field] = &[
    opt("name", null),
    req("input_type"),
    req("input_paths"),
    req("output_path"),
    opt("input_params", empty_map),
    opt("continue_on_compile_error", f),
    opt("output_type", yaml),
    opt("ignore_missing", f),
    opt("prune", t),
];

fn compile_fields(input_type: &str) -> Option<Vec<Field>> {
    let mut fields: Vec<Field> = COMPILE_BASE
        .iter()
        .map(|x| Field {
            name: x.name,
            default: x.default,
        })
        .collect();
    let mut set = |name: &'static str, default: fn() -> Value| {
        if let Some(existing) = fields.iter_mut().find(|x| x.name == name) {
            existing.default = Some(default);
        } else {
            fields.push(opt(name, default));
        }
    };
    match input_type {
        "jsonnet" => set("prune", f),
        "external" => {
            set("env_vars", empty_map);
            set("args", empty_list);
        }
        "copy" => {}
        "jinja2" => {
            set("output_type", || Value::Str("plain".into()));
            set("ignore_missing", t);
            set("suffix_remove", f);
            set("suffix_stripped", || Value::Str(".j2".into()));
        }
        "helm" => {
            set("output_type", || Value::Str("auto".into()));
            set("prune", f);
            set("helm_params", empty_map);
            set("helm_values", empty_map);
            set("helm_values_files", empty_list);
            set("helm_path", null);
            set("kube_version", null);
        }
        "kadet" => {
            set("input_value", null);
            set("prune", f);
        }
        "remove" => {
            if let Some(x) = fields.iter_mut().find(|x| x.name == "output_path") {
                x.default = Some(null);
            }
        }
        "kustomize" => {
            set("namespace", null);
            set("prune", f);
            set("patches", empty_map);
            set("patches_strategic", empty_map);
            set("patches_json", empty_map);
        }
        "cuelang" => {
            set("input", null);
            set("input_fill_path", null);
            set("output_yield_path", null);
            set("output_filename", || Value::Str("output.yaml".into()));
        }
        _ => return None,
    }
    Some(fields)
}

fn dependency_fields(dep_type: &str) -> Option<Vec<Field>> {
    let mut fields = vec![
        req("type"),
        req("source"),
        req("output_path"),
        opt("force_fetch", f),
    ];
    match dep_type {
        "helm" => fields.extend([
            req("chart_name"),
            opt("version", null),
            opt("helm_path", null),
        ]),
        "git" => fields.extend([opt("ref", null), opt("subdir", null), opt("submodules", f)]),
        "http" | "https" => fields.push(opt("unpack", f)),
        "oci" => fields.extend([
            opt("subpath", null),
            opt("media_type", null),
            opt("insecure", f),
            opt("tls_verify", t),
        ]),
        _ => return None,
    }
    Some(fields)
}

const VAULT_ENV: &[Field] = &[
    opt("addr", null),
    opt("skip_verify", t),
    opt("client_key", null),
    opt("client_cert", null),
    opt("cacert", null),
    opt("capath", null),
    opt("namespace", null),
];

struct Ctx<'a> {
    target: &'a str,
}

impl Ctx<'_> {
    fn err(&self, node: &Node, path: &str, msg: String) -> Error {
        Error::new("inventory::invalid_kapitan_config", msg)
            .with_target(self.target)
            .with_path(path)
            .with_label(node.origin, "declared here")
    }

    /// Reorder `node` into pydantic field order, fill defaults, reject unknown keys.
    fn model(
        &self,
        node: &mut Node,
        fields: &[Field],
        path: &str,
        allow_extra: bool,
    ) -> Result<()> {
        let Value::Map(map) = &mut node.value else {
            return Err(self.err(
                node,
                path,
                format!("expected a mapping, found {}", node.value.type_name()),
            ));
        };
        let old = std::mem::take(map);
        let mut new = Map::with_capacity(old.len());
        let mut rest = old;
        for field in fields {
            match rest.shift_remove(field.name) {
                Some(v) => {
                    new.insert(field.name.to_string(), v);
                }
                None => match field.default {
                    Some(default) => {
                        new.insert(
                            field.name.to_string(),
                            Node::new(default(), Origin::SYNTHETIC),
                        );
                    }
                    None => {
                        return Err(self.err(
                            node,
                            path,
                            format!("missing required field `{}`", field.name),
                        ));
                    }
                },
            }
        }
        if !allow_extra && !rest.is_empty() {
            let keys: Vec<&String> = rest.keys().collect();
            let first = rest.values().next().unwrap().origin;
            return Err(Error::new(
                "inventory::invalid_kapitan_config",
                format!(
                    "unknown field(s) {} in {path}",
                    keys.iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_target(self.target)
            .with_path(path)
            .with_label(first, "not a known field"));
        }
        new.extend(rest);
        *map = new;
        Ok(())
    }
}

/// Apply the pydantic normalisation to a fully resolved parameters tree.
pub fn normalize(params: &mut Node, target: &str) -> Result<()> {
    let ctx = Ctx { target };
    let Value::Map(top) = &mut params.value else {
        return Ok(());
    };
    let Some(kapitan) = top.get_mut("kapitan") else {
        return Ok(());
    };
    if kapitan.value.is_null() {
        return Ok(());
    }
    let fields = [
        opt("compile", empty_list),
        opt("vars", empty_map),
        opt("labels", empty_map),
        opt("dependencies", empty_list),
        opt("target_full_path", || Value::Str(String::new())),
        opt("secrets", null),
        opt("validate", empty_list),
    ];
    ctx.model(kapitan, &fields, "kapitan", true)?;
    let map = kapitan.as_map_mut().unwrap();

    // compile: discriminated on input_type
    if let Some(compile) = map.get_mut("compile") {
        if let Value::List(items) = &mut compile.value {
            for (i, item) in items.iter_mut().enumerate() {
                let path = format!("kapitan.compile[{i}]");
                let input_type = match item.get("input_type").map(|n| &n.value) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => {
                        return Err(ctx.err(
                            item,
                            &path,
                            "compile entry is missing a valid `input_type`".into(),
                        ));
                    }
                };
                let Some(fields) = compile_fields(&input_type) else {
                    return Err(ctx.err(
                        item,
                        &path,
                        format!("unknown input_type `{input_type}` (expected one of jsonnet, jinja2, helm, kadet, copy, remove, external, kustomize, cuelang)"),
                    ));
                };
                ctx.model(item, &fields, &path, false)?;
                if input_type == "helm" {
                    let m = item.as_map_mut().unwrap();
                    m.insert(
                        "output_type".into(),
                        Node::new(Value::Str("auto".into()), Origin::SYNTHETIC),
                    );
                }
            }
        } else if !compile.value.is_null() {
            return Err(ctx.err(
                compile,
                "kapitan.compile",
                "`compile` must be a list".into(),
            ));
        }
    }
    // vars: KapitanEssentialVars (extra allowed, target first)
    if let Some(vars) = map.get_mut("vars") {
        if vars.value.is_null() {
            vars.value = empty_map();
        }
        ctx.model(vars, &[opt("target", null)], "kapitan.vars", true)?;
    }
    if let Some(labels) = map.get_mut("labels")
        && labels.value.is_null()
    {
        labels.value = empty_map();
    }
    // dependencies: discriminated on type
    if let Some(deps) = map.get_mut("dependencies")
        && let Value::List(items) = &mut deps.value
    {
        for (i, item) in items.iter_mut().enumerate() {
            let path = format!("kapitan.dependencies[{i}]");
            let dep_type = match item.get("type").map(|n| &n.value) {
                Some(Value::Str(s)) => s.clone(),
                _ => {
                    return Err(ctx.err(item, &path, "dependency is missing a valid `type`".into()));
                }
            };
            let Some(fields) = dependency_fields(&dep_type) else {
                return Err(ctx.err(item, &path, format!("unknown dependency type `{dep_type}`")));
            };
            ctx.model(item, &fields, &path, false)?;
        }
    }
    // secrets: KapitanReferenceConfig
    if let Some(secrets) = map.get_mut("secrets")
        && !secrets.value.is_null()
    {
        let fields = [
            opt("gpg", null),
            opt("awskms", null),
            opt("vaultkv", null),
            opt("gkms", null),
            opt("vaulttransit", null),
            opt("azkms", null),
        ];
        ctx.model(secrets, &fields, "kapitan.secrets", false)?;
        let m = secrets.as_map_mut().unwrap();
        let sub = |m: &mut Map, key: &str, fields: &[Field]| -> Result<()> {
            if let Some(n) = m.get_mut(key)
                && !n.value.is_null()
            {
                ctx.model(n, fields, &format!("kapitan.secrets.{key}"), false)?;
            }
            Ok(())
        };
        sub(m, "gpg", &[opt("recipients", empty_list)])?;
        for k in ["awskms", "gkms", "azkms"] {
            sub(m, k, &[req("key")])?;
        }
        let mut vaultkv: Vec<Field> = VAULT_ENV
            .iter()
            .map(|x| Field {
                name: x.name,
                default: x.default,
            })
            .collect();
        vaultkv.extend([
            opt("engine", || Value::Str("kv-v2".into())),
            opt("auth", null),
            opt("crypto_key", null),
            opt("always_latest", f),
            opt("mount", || Value::Str("secret".into())),
            opt("key", null),
        ]);
        sub(m, "vaultkv", &vaultkv)?;
        let mut transit: Vec<Field> = VAULT_ENV
            .iter()
            .map(|x| Field {
                name: x.name,
                default: x.default,
            })
            .collect();
        transit.extend([
            opt("engine", || Value::Str("transit".into())),
            opt("auth", null),
            opt("crypto_key", null),
            opt("always_latest", f),
            opt("mount", || Value::Str("transit".into())),
            opt("key", null),
        ]);
        sub(m, "vaulttransit", &transit)?;
    }
    Ok(())
}
