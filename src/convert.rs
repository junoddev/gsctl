//! GVariant ↔ JSON conversion, guided by the key's GVariant type.
//!
//! Mapping:
//! - `b` ↔ bool; `y n q i u x t` ↔ integer; `d` ↔ number; `s o g` ↔ string
//! - arrays and tuples ↔ JSON arrays; `ay` is an array of byte values
//!   (a string is accepted on input and stored as a NUL-terminated bytestring)
//! - dictionaries `a{..}` ↔ JSON objects; non-string keys are written as their
//!   JSON text (`{"1": ..}`)
//! - maybe `m..` ↔ `null` or the inner value
//! - `v` ↔ the inner value. On input its type is inferred (bool → b, integer →
//!   i or x or t, number → d, string → s, all-string array → as, other array →
//!   av, object → a{sv}), so a `v` holding e.g. a `u` comes back as an `i`.

use glib::{Variant, VariantClass, VariantTy, VariantType};
use serde_json::{Map, Number, Value};

/// Convert a GVariant to JSON.
pub fn to_json(v: &Variant) -> Value {
    match v.classify() {
        VariantClass::Boolean => Value::Bool(v.get::<bool>().unwrap()),
        VariantClass::Byte => v.get::<u8>().unwrap().into(),
        VariantClass::Int16 => v.get::<i16>().unwrap().into(),
        VariantClass::Uint16 => v.get::<u16>().unwrap().into(),
        VariantClass::Int32 => v.get::<i32>().unwrap().into(),
        VariantClass::Uint32 => v.get::<u32>().unwrap().into(),
        VariantClass::Int64 => v.get::<i64>().unwrap().into(),
        VariantClass::Uint64 => v.get::<u64>().unwrap().into(),
        VariantClass::Handle => v.get::<glib::variant::Handle>().map(|h| h.0).unwrap_or_default().into(),
        VariantClass::Double => Number::from_f64(v.get::<f64>().unwrap()).map_or(Value::Null, Value::Number),
        VariantClass::String | VariantClass::ObjectPath | VariantClass::Signature => {
            Value::String(v.str().unwrap_or_default().to_string())
        }
        VariantClass::Variant => to_json(&v.as_variant().unwrap()),
        VariantClass::Maybe => v.as_maybe().map_or(Value::Null, |c| to_json(&c)),
        VariantClass::Array if v.type_().element().is_dict_entry() => {
            let mut map = Map::new();
            for entry in v.iter() {
                let k = entry.child_value(0);
                let key = match k.str() {
                    Some(s) => s.to_string(),
                    None => to_json(&k).to_string(),
                };
                map.insert(key, to_json(&entry.child_value(1)));
            }
            Value::Object(map)
        }
        VariantClass::Array | VariantClass::Tuple | VariantClass::DictEntry => {
            Value::Array(v.iter().map(|c| to_json(&c)).collect())
        }
        _ => Value::String(v.print(true).to_string()),
    }
}

/// Convert JSON to a GVariant of type `ty`.
pub fn from_json(j: &Value, ty: &VariantTy) -> Result<Variant, String> {
    let mismatch = || format!("expected {}, got {}", describe(ty), json_kind(j));
    let int = |min: i128, max: i128| -> Result<i128, String> {
        let n = j.as_i64().map(i128::from).or_else(|| j.as_u64().map(i128::from)).ok_or_else(mismatch)?;
        if n < min || n > max {
            return Err(format!("{n} is out of range for {} ({min}..={max})", describe(ty)));
        }
        Ok(n)
    };
    let s = ty.as_str();
    Ok(match s {
        "b" => j.as_bool().ok_or_else(mismatch)?.into(),
        "y" => (int(0, u8::MAX.into())? as u8).into(),
        "n" => (int(i16::MIN.into(), i16::MAX.into())? as i16).into(),
        "q" => (int(0, u16::MAX.into())? as u16).into(),
        "i" => (int(i32::MIN.into(), i32::MAX.into())? as i32).into(),
        "u" => (int(0, u32::MAX.into())? as u32).into(),
        "x" => (int(i64::MIN.into(), i64::MAX.into())? as i64).into(),
        "t" => (int(0, u64::MAX.into())? as u64).into(),
        "d" => {
            let d = j.as_f64().ok_or_else(mismatch)?;
            if !d.is_finite() {
                return Err(format!("{d} is not allowed: numbers must be finite"));
            }
            d.into()
        }
        "s" => j.as_str().ok_or_else(mismatch)?.into(),
        "o" | "g" => {
            let text = j.as_str().ok_or_else(mismatch)?;
            let quoted = Variant::from(text).print(false);
            Variant::parse(Some(ty), &quoted).map_err(|_| format!("{text:?} is not a valid {}", describe(ty)))?
        }
        "v" => Variant::from_variant(&infer(j)?),
        "ay" => match j {
            Value::String(text) => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                Variant::array_from_fixed_array(&bytes)
            }
            _ => array(j, ty)?,
        },
        _ if ty.is_maybe() => match j {
            Value::Null => Variant::from_none(ty.element()),
            _ => Variant::from_some(&from_json(j, ty.element())?),
        },
        _ if ty.is_array() && ty.element().is_dict_entry() => dict(j, ty)?,
        _ if ty.is_array() => array(j, ty)?,
        _ if ty.is_tuple() || ty.is_dict_entry() => {
            let items = j.as_array().ok_or_else(mismatch)?;
            let types: Vec<&VariantTy> =
                if ty.is_dict_entry() { vec![ty.key(), ty.value()] } else { ty.tuple_types().collect() };
            if items.len() != types.len() {
                return Err(format!("expected {} with {} items, got {}", describe(ty), types.len(), items.len()));
            }
            let children = items
                .iter()
                .zip(types)
                .enumerate()
                .map(|(i, (item, t))| from_json(item, t).map_err(|e| format!("item {i}: {e}")))
                .collect::<Result<Vec<_>, _>>()?;
            if ty.is_dict_entry() {
                Variant::from_dict_entry(&children[0], &children[1])
            } else {
                Variant::tuple_from_iter(children)
            }
        }
        _ => return Err(format!("unsupported type {s}")),
    })
}

fn array(j: &Value, ty: &VariantTy) -> Result<Variant, String> {
    let items = j.as_array().ok_or_else(|| format!("expected {}, got {}", describe(ty), json_kind(j)))?;
    let elem = ty.element();
    let children = items
        .iter()
        .enumerate()
        .map(|(i, item)| from_json(item, elem).map_err(|e| format!("item {i}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Variant::array_from_iter_with_type(elem, children))
}

fn dict(j: &Value, ty: &VariantTy) -> Result<Variant, String> {
    let entry = ty.element();
    let (kt, vt) = (entry.key(), entry.value());
    let pairs: Vec<(Value, &Value)> = match j {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let key = if matches!(kt.as_str(), "s" | "o" | "g") {
                    Value::String(k.clone())
                } else {
                    serde_json::from_str(k).unwrap_or_else(|_| Value::String(k.clone()))
                };
                (key, v)
            })
            .collect(),
        // Also accept [[key, value], ...].
        Value::Array(items) => items
            .iter()
            .map(|item| match item.as_array().map(Vec::as_slice) {
                Some([k, v]) => Ok((k.clone(), v)),
                _ => Err(format!("expected [key, value] pairs for {}", describe(ty))),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(format!("expected {}, got {}", describe(ty), json_kind(j))),
    };
    let children = pairs
        .iter()
        .map(|(k, v)| {
            let key = from_json(k, kt).map_err(|e| format!("key {k}: {e}"))?;
            let val = from_json(v, vt).map_err(|e| format!("value for {k}: {e}"))?;
            Ok(Variant::from_dict_entry(&key, &val))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Variant::array_from_iter_with_type(entry, children))
}

/// Pick a GVariant type for JSON going into a `v`.
fn infer(j: &Value) -> Result<Variant, String> {
    Ok(match j {
        Value::Null => return Err("null has no GVariant type inside a variant".into()),
        Value::Bool(b) => b.into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i32::try_from(i) {
                    Ok(i) => i.into(),
                    Err(_) => i.into(),
                }
            } else if let Some(u) = n.as_u64() {
                u.into()
            } else {
                n.as_f64().unwrap().into()
            }
        }
        Value::String(s) => s.as_str().into(),
        Value::Array(items) if items.iter().all(Value::is_string) => from_json(j, VariantTy::STRING_ARRAY)?,
        Value::Array(items) => {
            let children =
                items.iter().map(|i| infer(i).map(|v| Variant::from_variant(&v))).collect::<Result<Vec<_>, _>>()?;
            Variant::array_from_iter_with_type(VariantTy::VARIANT, children)
        }
        Value::Object(_) => from_json(j, VariantTy::VARDICT)?,
    })
}

/// `Err` if `v` holds a NaN or infinite double anywhere, including inside
/// containers and variants. GSettings would store them, but they pass range
/// checks (NaN compares false) and break JSON output.
pub fn check_finite(v: &Variant) -> Result<(), String> {
    match v.classify() {
        VariantClass::Double => {
            let d = v.get::<f64>().unwrap();
            if d.is_finite() { Ok(()) } else { Err(format!("{d} is not allowed: numbers must be finite")) }
        }
        VariantClass::Variant => check_finite(&v.as_variant().unwrap()),
        VariantClass::Maybe => v.as_maybe().map_or(Ok(()), |c| check_finite(&c)),
        VariantClass::Array | VariantClass::Tuple | VariantClass::DictEntry => {
            let sig = v.type_().as_str();
            if sig.contains('d') || sig.contains('v') { v.iter().try_for_each(|c| check_finite(&c)) } else { Ok(()) }
        }
        _ => Ok(()),
    }
}

/// [`Variant::parse`] that also rejects NaN and infinity.
pub fn parse_finite(ty: &VariantTy, text: &str) -> Result<Variant, String> {
    let v = Variant::parse(Some(ty), text).map_err(|e| e.to_string())?;
    check_finite(&v)?;
    Ok(v)
}

/// Parse a command-line value for a key of type `ty`.
///
/// Tried in order: JSON (`true`, `42`, `"text"`, `["a","b"]`), GVariant text
/// syntax (`@as []`, `('xkb', 'us')`, `uint32 5`), and, for string types, the
/// raw text itself, so `prefer-dark` works without quotes. NaN and infinity
/// are rejected.
pub fn parse_input(raw: &str, ty: &VariantTy) -> Result<Variant, String> {
    let v = parse_any(raw, ty)?;
    check_finite(&v)?;
    Ok(v)
}

fn parse_any(raw: &str, ty: &VariantTy) -> Result<Variant, String> {
    let json_err = match serde_json::from_str::<Value>(raw) {
        Ok(j) => match from_json(&j, ty) {
            Ok(v) => return Ok(v),
            Err(e) => Some(e),
        },
        Err(_) => None,
    };
    let gvariant_err = match Variant::parse(Some(ty), raw) {
        Ok(v) => return Ok(v),
        // GLib prefixes a character range ("0-3:unknown keyword").
        Err(e) => e.message().split_once(':').map_or(e.message(), |(_, m)| m).to_string(),
    };
    if ty == VariantTy::STRING {
        return Ok(raw.into());
    }
    if ty.as_str() == "ms" {
        return Ok(Variant::from_some(&raw.into()));
    }
    Err(json_err
        .unwrap_or_else(|| format!("{raw:?} is not valid JSON or GVariant text for {} ({gvariant_err})", describe(ty))))
}

/// A short English description of a GVariant type, for error messages.
pub fn describe(ty: &VariantTy) -> String {
    let s = ty.as_str();
    let basic = match s {
        "b" => Some("boolean"),
        "y" => Some("byte (0..=255)"),
        "n" => Some("16-bit integer"),
        "q" => Some("unsigned 16-bit integer"),
        "i" => Some("32-bit integer"),
        "u" => Some("unsigned 32-bit integer"),
        "x" => Some("64-bit integer"),
        "t" => Some("unsigned 64-bit integer"),
        "h" => Some("file handle"),
        "d" => Some("number"),
        "s" => Some("string"),
        "o" => Some("D-Bus object path"),
        "g" => Some("D-Bus signature"),
        "v" => Some("variant (any value)"),
        "ay" => Some("byte array"),
        _ => None,
    };
    if let Some(b) = basic {
        return b.to_string();
    }
    if ty.is_maybe() {
        format!("{} or null", describe(ty.element()))
    } else if ty.is_array() && ty.element().is_dict_entry() {
        let e = ty.element();
        format!("object mapping {} to {}", describe(e.key()), describe(e.value()))
    } else if ty.is_array() {
        format!("array of {}", plural(&describe(ty.element())))
    } else if ty.is_tuple() {
        let parts: Vec<String> = ty.tuple_types().map(describe).collect();
        format!("tuple [{}]", parts.join(", "))
    } else {
        s.to_string()
    }
}

fn plural(s: &str) -> String {
    match s {
        "boolean" | "string" | "number" => format!("{s}s"),
        _ => s.to_string(),
    }
}

fn json_kind(j: &Value) -> &'static str {
    match j {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Owned type from a signature string; panics on invalid input (tests only).
pub fn ty(s: &str) -> VariantType {
    VariantType::new(s).unwrap_or_else(|_| panic!("invalid type {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// GVariant text → JSON → GVariant must give back the same value, and the
    /// JSON must look like `expected`.
    fn round_trip(sig: &str, gvariant: &str, expected: Value) {
        let t = ty(sig);
        let v = Variant::parse(Some(&t), gvariant).unwrap();
        let j = to_json(&v);
        assert_eq!(j, expected, "{sig} {gvariant} → JSON");
        let back = from_json(&j, &t).unwrap_or_else(|e| panic!("{sig} {j} → GVariant: {e}"));
        assert_eq!(back, v, "{sig}: {} vs {}", back.print(true), v.print(true));
        assert_eq!(back.type_().as_str(), sig);
    }

    #[test]
    fn basic_types() {
        round_trip("b", "true", json!(true));
        round_trip("b", "false", json!(false));
        round_trip("y", "255", json!(255));
        round_trip("n", "-32768", json!(-32768));
        round_trip("q", "65535", json!(65535));
        round_trip("i", "-5", json!(-5));
        round_trip("u", "4294967295", json!(4294967295u32));
        round_trip("x", "-9223372036854775808", json!(i64::MIN));
        round_trip("t", "18446744073709551615", json!(u64::MAX));
        round_trip("d", "1.25", json!(1.25));
        round_trip("d", "2.0", json!(2.0));
        round_trip("s", "'hello'", json!("hello"));
        round_trip("s", "''", json!(""));
        round_trip("o", "'/org/gnome/foo'", json!("/org/gnome/foo"));
        round_trip("g", "'a{sv}'", json!("a{sv}"));
    }

    #[test]
    fn containers() {
        round_trip("as", "['a', 'b']", json!(["a", "b"]));
        round_trip("as", "@as []", json!([]));
        round_trip("ai", "[1, 2, 3]", json!([1, 2, 3]));
        round_trip("ay", "[byte 1, 2]", json!([1, 2]));
        round_trip("a{ss}", "{'a': 'b', 'c': 'd'}", json!({"a": "b", "c": "d"}));
        round_trip("a{sas}", "{'k': ['x']}", json!({"k": ["x"]}));
        round_trip("a{is}", "{1: 'one'}", json!({"1": "one"}));
        round_trip("(ii)", "(1, 2)", json!([1, 2]));
        round_trip("(sdb)", "('x', 1.5, true)", json!(["x", 1.5, true]));
        round_trip("a(ss)", "[('xkb', 'us'), ('ibus', 'mozc')]", json!([["xkb", "us"], ["ibus", "mozc"]]));
        round_trip("a(sa{sv})", "[('k', {'n': <1>})]", json!([["k", {"n": 1}]]));
        round_trip("()", "()", json!([]));
    }

    #[test]
    fn variants_with_inferable_types() {
        round_trip(
            "a{sv}",
            "{'b': <true>, 'i': <5>, 'd': <0.5>, 's': <'x'>, 'as': <['p']>, 'big': <int64 5000000000>}",
            json!({"b": true, "i": 5, "d": 0.5, "s": "x", "as": ["p"], "big": 5000000000i64}),
        );
        round_trip("a{sv}", "{'nested': <{'a': <1>}>}", json!({"nested": {"a": 1}}));
        round_trip("v", "<'x'>", json!("x"));
    }

    #[test]
    fn maybe_types() {
        round_trip("ms", "nothing", json!(null));
        round_trip("ms", "just 'x'", json!("x"));
        round_trip("mi", "just 3", json!(3));
        round_trip("mas", "just ['a']", json!(["a"]));
    }

    #[test]
    fn variant_loses_narrow_integer_types() {
        // Documented limitation: a v holding a u comes back as an i.
        let v = Variant::parse(Some(&ty("v")), "<uint32 7>").unwrap();
        let back = from_json(&to_json(&v), &ty("v")).unwrap();
        assert_eq!(back.as_variant().unwrap().type_().as_str(), "i");
    }

    #[test]
    fn bytestring_input() {
        let v = from_json(&json!("abc"), &ty("ay")).unwrap();
        assert_eq!(v.print(false).as_str(), "b'abc'");
    }

    #[test]
    fn dict_accepts_pairs() {
        let v = from_json(&json!([["a", "b"]]), &ty("a{ss}")).unwrap();
        assert_eq!(v.print(false).as_str(), "{'a': 'b'}");
    }

    #[test]
    fn rejects_mismatches_with_clear_messages() {
        let err = |sig: &str, j: Value| from_json(&j, &ty(sig)).unwrap_err();
        assert_eq!(err("b", json!("yes")), "expected boolean, got string");
        assert_eq!(err("as", json!("a")), "expected array of strings, got string");
        assert_eq!(err("as", json!(["a", 1])), "item 1: expected string, got integer");
        assert!(err("y", json!(256)).contains("out of range"));
        assert!(err("u", json!(-1)).contains("out of range"));
        assert!(err("i", json!(1.5)).contains("expected 32-bit integer, got number"));
        assert!(err("(ii)", json!([1])).contains("with 2 items"));
        assert!(err("o", json!("not a path")).contains("not a valid D-Bus object path"));
    }

    #[test]
    fn parse_input_accepts_json_gvariant_and_bare_strings() {
        let p = |raw: &str, sig: &str| parse_input(raw, &ty(sig)).map(|v| v.print(true).to_string());
        assert_eq!(p("true", "b").unwrap(), "true");
        assert_eq!(p("42", "i").unwrap(), "42");
        assert_eq!(p("-3", "i").unwrap(), "-3");
        assert_eq!(p(r#"["a","b"]"#, "as").unwrap(), "['a', 'b']");
        assert_eq!(p("['a', 'b']", "as").unwrap(), "['a', 'b']");
        assert_eq!(p("@as []", "as").unwrap(), "@as []");
        assert_eq!(p("('xkb', 'us')", "(ss)").unwrap(), "('xkb', 'us')");
        assert_eq!(p(r#""quoted""#, "s").unwrap(), "'quoted'");
        assert_eq!(p("'single'", "s").unwrap(), "'single'");
        assert_eq!(p("prefer-dark", "s").unwrap(), "'prefer-dark'");
        assert_eq!(p("true", "s").unwrap(), "'true'");
        assert_eq!(p("null", "ms").unwrap(), "@ms nothing");
        assert_eq!(p("label", "ms").unwrap(), "@ms 'label'");
        assert!(p("maybe", "b").is_err());
        assert_eq!(p(r#"["a",1]"#, "as").unwrap_err(), "item 1: expected string, got integer");
    }

    #[test]
    fn rejects_non_finite_numbers() {
        let p = |raw: &str, sig: &str| parse_input(raw, &ty(sig));
        for (raw, sig) in [
            ("nan", "d"),
            ("inf", "d"),
            ("-inf", "d"),
            ("[1.0, nan]", "ad"),
            ("('x', inf, true)", "(sdb)"),
            ("{'a': <nan>}", "a{sv}"),
            ("just inf", "md"),
        ] {
            assert!(p(raw, sig).unwrap_err().contains("finite"), "{raw} as {sig}");
        }
        // Out of range for a double: GLib's own parser refuses it.
        assert!(p("1e999", "d").is_err());
        assert!(p("1.5", "d").is_ok());
        assert!(p("{'a': <1.5>, 'b': <'x'>}", "a{sv}").is_ok());
        assert!(parse_finite(&ty("d"), "nan").is_err());
        assert!(parse_finite(&ty("d"), "2.5").is_ok());
    }

    #[test]
    fn descriptions() {
        assert_eq!(describe(&ty("as")), "array of strings");
        assert_eq!(describe(&ty("a{sv}")), "object mapping string to variant (any value)");
        assert_eq!(describe(&ty("a(ss)")), "array of tuple [string, string]");
        assert_eq!(describe(&ty("ms")), "string or null");
    }
}
