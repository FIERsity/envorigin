//! YAML parsing with the features real-world CI files rely on but
//! `serde_yaml::from_str` rejects:
//!
//! - **Multiple documents** (`---` separators): GitLab CI merges documents
//!   in order, later keys winning (e.g. glab's `.gitlab-ci.yml` starts with
//!   a `spec:` document followed by the pipeline document).
//! - **Several `<<` merge keys in one mapping**: YAML 1.1 allows multiple
//!   merge keys; `serde_yaml` rejects the duplicate key outright, and real
//!   GitLab configs (e.g. fdroid's) use two merge keys per job.
//!
//! Duplicate `<<` keys are folded into a single sequence-valued merge
//! (`<<: [*a, *b]`), preserving order. YAML 1.1 merge semantics apply:
//! explicit keys beat merge keys; earlier merges beat later ones.
//! Anchors are resolved by `serde_yaml` itself.

use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_yaml::{Mapping, Value};

/// Parse YAML content, merging multiple documents and resolving every
/// `<<` merge key. The result is a single value tree.
pub fn parse_merged(content: &str) -> Result<Value, serde_yaml::Error> {
    let mut documents: Vec<Value> = serde_yaml::Deserializer::from_str(content)
        .map(|document| Tolerant.deserialize(document))
        .collect::<Result<_, _>>()?;
    if documents.is_empty() {
        return Ok(Value::Null);
    }
    let mut merged = documents.remove(0);
    for document in documents {
        merged = merge_values(merged, document);
    }
    resolve_merge_keys(&mut merged);
    Ok(merged)
}

/// A `DeserializeSeed` for `Value` that tolerates duplicate mapping keys:
/// several `<<` merge keys fold into one sequence-valued merge; any other
/// duplicate key keeps the last value (matching the lenient parsers GitLab
/// and Docker use).
struct Tolerant;

impl<'de> DeserializeSeed<'de> for Tolerant {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(TolerantVisitor)
    }
}

struct TolerantVisitor;

impl<'de> Visitor<'de> for TolerantVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any YAML value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Value::Number(serde_yaml::Number::from(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Tolerant.deserialize(deserializer)
    }

    fn visit_enum<A>(self, data: A) -> Result<Value, A::Error>
    where
        A: serde::de::EnumAccess<'de>,
    {
        use serde::de::VariantAccess;
        // Unknown tags (`!reference`, `!policy`, ...) surface as enum
        // variants with the tagged value as a newtype. The tag name is not
        // meaningful for environment analysis; keep the tagged value.
        let (_, access) = data.variant_seed(Tolerant)?;
        access.newtype_variant_seed(Tolerant)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Tolerant)? {
            values.push(value);
        }
        Ok(Value::Sequence(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut mapping = Mapping::new();
        while let Some((key, value)) = map.next_entry_seed(Tolerant, Tolerant)? {
            if key.as_str() == Some("<<") {
                match mapping.get_mut(&key) {
                    Some(existing) => {
                        // Fold duplicate merge keys into one sequence,
                        // preserving order (earlier merges win).
                        let mut merged = match existing.clone() {
                            Value::Sequence(sequence) => sequence,
                            other => vec![other],
                        };
                        merged.push(value);
                        mapping.insert(key, Value::Sequence(merged));
                    }
                    None => {
                        mapping.insert(key, value);
                    }
                }
            } else {
                mapping.insert(key, value);
            }
        }
        Ok(Value::Mapping(mapping))
    }
}

/// Recursively merge `over` into `base`: mappings merge key-by-key (later
/// documents win), everything else is replaced by the later value.
fn merge_values(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Mapping(mut base), Value::Mapping(over)) => {
            for (key, value) in over {
                match base.remove(&key) {
                    Some(existing) => {
                        base.insert(key, merge_values(existing, value));
                    }
                    None => {
                        base.insert(key, value);
                    }
                }
            }
            Value::Mapping(base)
        }
        (_, over) => over,
    }
}

/// Replace every `<<` merge key by its merged contents. Explicit keys win
/// over any merge; with several merge keys the earlier one wins (YAML 1.1
/// sequence-merge semantics).
fn resolve_merge_keys(value: &mut Value) {
    match value {
        Value::Mapping(mapping) => {
            let merges: Vec<Value> = mapping
                .iter()
                .filter(|(key, _)| key.as_str() == Some("<<"))
                .map(|(_, merge)| merge.clone())
                .collect();
            if !merges.is_empty() {
                let mut merged = Value::Mapping(Mapping::new());
                // Process in reverse so earlier merges override later ones.
                for merge in merges.iter().rev() {
                    match merge {
                        Value::Mapping(_) => {
                            let mut merge = merge.clone();
                            resolve_merge_keys(&mut merge);
                            merged = merge_values(merged, merge);
                        }
                        Value::Sequence(sequence) => {
                            for item in sequence.iter().rev() {
                                let mut item = item.clone();
                                resolve_merge_keys(&mut item);
                                if let Value::Mapping(_) = item {
                                    merged = merge_values(merged, item);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let explicit: Mapping = mapping
                    .iter()
                    .filter(|(key, _)| key.as_str() != Some("<<"))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                let final_mapping = merge_values(merged, Value::Mapping(explicit));
                if let Value::Mapping(final_mapping) = final_mapping {
                    *mapping = final_mapping;
                }
            }
            for (_, value) in mapping.iter_mut() {
                resolve_merge_keys(value);
            }
        }
        Value::Sequence(sequence) => {
            for item in sequence.iter_mut() {
                resolve_merge_keys(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_multiple_documents_later_wins() {
        let value = parse_merged("a: 1\n---\nb: 2\n---\na: 3\n").unwrap();
        let mapping = value.as_mapping().unwrap();
        assert_eq!(mapping.get("a").and_then(Value::as_i64), Some(3));
        assert_eq!(mapping.get("b").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn merges_nested_mappings_across_documents() {
        let value = parse_merged("job:\n  env:\n    A: 1\n---\njob:\n  env:\n    B: 2\n").unwrap();
        let env = value["job"]["env"].as_mapping().unwrap();
        assert_eq!(env.get("A").and_then(Value::as_i64), Some(1));
        assert_eq!(env.get("B").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn resolves_single_merge_key() {
        let value = parse_merged(
            "base: &b\n  image: debian\n  tags: [x]\njob:\n  <<: *b\n  script: echo hi\n",
        )
        .unwrap();
        let job = value["job"].as_mapping().unwrap();
        assert_eq!(job.get("image").and_then(Value::as_str), Some("debian"));
        assert_eq!(job.get("script").and_then(Value::as_str), Some("echo hi"));
        // the merge key itself is gone
        assert!(!job.contains_key(&Value::String("<<".to_string())));
    }

    #[test]
    fn resolves_two_merge_keys() {
        // fdroid pattern: two << in one map.
        let value = parse_merged(
            "first: &f\n  image: one\ntags: &t\n  tags: [x]\njob:\n  <<: *f\n  <<: *t\n  script: run\n",
        )
        .unwrap();
        let job = value["job"].as_mapping().unwrap();
        assert_eq!(job.get("image").and_then(Value::as_str), Some("one"));
        assert_eq!(
            job.get("tags").and_then(Value::as_sequence).unwrap()[0].as_str(),
            Some("x")
        );
        assert_eq!(job.get("script").and_then(Value::as_str), Some("run"));
    }

    #[test]
    fn earlier_merge_wins_on_conflict() {
        let value = parse_merged(
            "first: &f\n  rules: [first]\nsecond: &s\n  rules: [second]\njob:\n  <<: *f\n  <<: *s\n",
        )
        .unwrap();
        let job = value["job"].as_mapping().unwrap();
        assert_eq!(
            job.get("rules").and_then(Value::as_sequence).unwrap()[0].as_str(),
            Some("first")
        );
    }

    #[test]
    fn explicit_keys_win_over_merge() {
        let value =
            parse_merged("base: &b\n  image: debian\njob:\n  <<: *b\n  image: custom\n").unwrap();
        assert_eq!(value["job"]["image"].as_str(), Some("custom"));
    }

    #[test]
    fn merges_sequence_of_merge_values() {
        let value =
            parse_merged("one: &o\n  a: 1\ntwo: &t\n  b: 2\njob:\n  <<: [*o, *t]\n").unwrap();
        let job = value["job"].as_mapping().unwrap();
        assert_eq!(job.get("a").and_then(Value::as_i64), Some(1));
        assert_eq!(job.get("b").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn keeps_unknown_tagged_values() {
        // GitLab `!reference` and `!policy` custom tags.
        let value = parse_merged(
            "job:\n  before_script:\n    - !reference [.template, before_script]\n    - echo hi\n",
        )
        .unwrap();
        let script = value["job"]["before_script"].as_sequence().unwrap();
        assert_eq!(
            script[0].as_sequence().unwrap()[0].as_str(),
            Some(".template")
        );
        assert_eq!(script[1].as_str(), Some("echo hi"));
    }

    #[test]
    fn resolves_chained_merge_keys() {
        let value = parse_merged(
            "base: &b\n  image: debian\n  tags: [x]\nmid: &m\n  <<: *b\n  script: build\njob:\n  <<: *m\n",
        )
        .unwrap();
        let job = value["job"].as_mapping().unwrap();
        assert_eq!(job.get("image").and_then(Value::as_str), Some("debian"));
        assert_eq!(job.get("script").and_then(Value::as_str), Some("build"));
    }
}
