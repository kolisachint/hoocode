//! A small model of the JavaScript values highlight.js grammars are made of.
//!
//! highlight.js compiles a language by mutating its mode objects in place,
//! and modes and keyword tables are shared between parents; the compiled
//! result depends on that sharing (a keywords object's `$pattern` is deleted
//! by the first mode that compiles it). So grammars are kept as shared,
//! mutable objects with JavaScript's identity, truthiness and frozen-object
//! rules, and the engine is ported against them.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hoocode_tui_util::js_regex::JsRegex;

use crate::engine::ResumableMultiRegex;

/// A named callback (`on:begin`, `on:end`, a compiler extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callback {
    EndSameAsBeginBegin,
    EndSameAsBeginEnd,
    ShebangBegin,
    JsxIsTrulyOpeningTag,
    MathematicaSystemSymbol,
    SkipIfHasPrecedingDot,
    ExtBeforeMatch,
}

impl Callback {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "endSameAsBegin.begin" => Self::EndSameAsBeginBegin,
            "endSameAsBegin.end" => Self::EndSameAsBeginEnd,
            "shebang.begin" => Self::ShebangBegin,
            "jsx.isTrulyOpeningTag" => Self::JsxIsTrulyOpeningTag,
            "mathematica.systemSymbol" => Self::MathematicaSystemSymbol,
            "ext.beforeMatch" => Self::ExtBeforeMatch,
            _ => return None,
        })
    }
}

/// A regex literal: its source and flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReLit {
    pub source: String,
    pub flags: String,
}

/// A compiled regex with the source it came from.
pub struct CompiledRe {
    pub source: String,
    pub re: JsRegex,
}

impl std::fmt::Debug for CompiledRe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{}/", self.source)
    }
}

pub type Obj = Rc<RefCell<ObjData>>;
pub type Arr = Rc<RefCell<Vec<V>>>;

#[derive(Clone, Debug)]
pub enum V {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    Re(Rc<ReLit>),
    Arr(Arr),
    Obj(Obj),
    Fn(Callback),
    /// A compiled regex the engine stored on a mode.
    Compiled(Rc<CompiledRe>),
    /// A mode's scanner.
    Matcher(Rc<RefCell<ResumableMultiRegex>>),
    /// A compiled keyword: `[className, relevance]`.
    Keyword(Rc<str>, f64),
}

#[derive(Debug, Default)]
pub struct ObjData {
    props: Vec<(String, V)>,
    pub frozen: bool,
}

impl ObjData {
    pub fn keys(&self) -> Vec<String> {
        self.props.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn entries(&self) -> Vec<(String, V)> {
        self.props.clone()
    }
}

pub fn new_obj() -> Obj {
    Rc::new(RefCell::new(ObjData::default()))
}

pub fn new_arr(items: Vec<V>) -> V {
    V::Arr(Rc::new(RefCell::new(items)))
}

/// `obj[key]`.
pub fn get(obj: &Obj, key: &str) -> V {
    obj.borrow()
        .props
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or(V::Undefined)
}

/// `obj[key] = value` (ignored on a frozen object, as in sloppy mode).
pub fn set(obj: &Obj, key: &str, value: V) {
    let mut o = obj.borrow_mut();
    if o.frozen {
        return;
    }
    if let Some(slot) = o.props.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value;
    } else {
        o.props.push((key.to_string(), value));
    }
}

/// `delete obj[key]` (ignored on a frozen object).
pub fn delete(obj: &Obj, key: &str) {
    let mut o = obj.borrow_mut();
    if o.frozen {
        return;
    }
    o.props.retain(|(k, _)| k != key);
}

/// `inherit(original, ...objects)`: a fresh object with every key copied.
pub fn inherit(original: &Obj, others: &[&Obj]) -> Obj {
    let result = new_obj();
    for (k, v) in original.borrow().props.iter() {
        set(&result, k, v.clone());
    }
    for other in others {
        for (k, v) in other.borrow().props.iter() {
            set(&result, k, v.clone());
        }
    }
    result
}

impl V {
    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            V::Undefined | V::Null => false,
            V::Bool(b) => *b,
            V::Num(n) => *n != 0.0 && !n.is_nan(),
            V::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, V::Undefined)
    }

    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            V::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            V::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            V::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<Vec<V>> {
        match self {
            V::Arr(a) => Some(a.borrow().clone()),
            _ => None,
        }
    }

    /// `typeof value === "object"` (arrays, objects and `null`).
    pub fn is_js_object(&self) -> bool {
        matches!(self, V::Arr(_) | V::Obj(_) | V::Null)
    }
}

pub fn str_v(s: &str) -> V {
    V::Str(Rc::from(s))
}

// ---- loading the grammars --------------------------------------------------

fn decode(value: &serde_json::Value, nodes: &HashMap<u64, Obj>) -> V {
    use serde_json::Value as J;
    match value {
        J::Null => V::Null,
        J::Bool(b) => V::Bool(*b),
        J::Number(n) => V::Num(n.as_f64().unwrap_or(f64::NAN)),
        J::String(s) => str_v(s),
        J::Array(items) => new_arr(items.iter().map(|v| decode(v, nodes)).collect()),
        J::Object(map) => {
            if let Some(id) = map.get("$ref").and_then(J::as_u64) {
                return V::Obj(nodes[&id].clone());
            }
            if let Some(items) = map.get("$arr").and_then(J::as_array) {
                return new_arr(items.iter().map(|v| decode(v, nodes)).collect());
            }
            if let Some(source) = map.get("$re").and_then(J::as_str) {
                return V::Re(Rc::new(ReLit {
                    source: source.to_string(),
                    flags: map
                        .get("flags")
                        .and_then(J::as_str)
                        .unwrap_or("")
                        .to_string(),
                }));
            }
            if let Some(name) = map.get("$fn").and_then(J::as_str) {
                return match Callback::from_name(name) {
                    Some(cb) => V::Fn(cb),
                    None => V::Undefined,
                };
            }
            V::Undefined
        }
    }
}

/// Decodes one language's chunk (format: `tests/it/grammar_index.rs`) into the
/// node table and returns the language's root object.
///
/// A node is one object per table, whichever chunk loads it first: a node
/// shared by two languages stays one object, as when the whole dump was
/// decoded at once. A chunk holds every node reachable from its root, so each
/// `$ref` resolves within the table.
pub fn load_chunk(json: &str, root: u64, table: &mut HashMap<u64, Obj>) -> Obj {
    let doc: serde_json::Value = serde_json::from_str(json).expect("grammar chunk");
    let raw_nodes = doc["nodes"].as_object().expect("nodes");
    let mut fresh = HashSet::new();
    for key in raw_nodes.keys() {
        let id: u64 = key.parse().expect("node id");
        if let Entry::Vacant(slot) = table.entry(id) {
            slot.insert(new_obj());
            fresh.insert(id);
        }
    }
    for (key, raw) in raw_nodes {
        let id: u64 = key.parse().expect("node id");
        if !fresh.contains(&id) {
            continue;
        }
        let node = table[&id].clone();
        let mut props = Vec::new();
        for (k, v) in raw["props"].as_object().expect("props") {
            let value = decode(v, table);
            // JSON has no `undefined`; the dumper wrote it as null, and
            // `undefined`-valued keys behave as absent for the engine.
            if matches!(value, V::Null) && v.is_null() {
                props.push((k.clone(), V::Undefined));
                continue;
            }
            props.push((k.clone(), value));
        }
        let mut data = node.borrow_mut();
        data.props = props;
        data.frozen = raw["frozen"].as_bool().unwrap_or(false);
    }
    table[&root].clone()
}
