pub const DIRECT_MANY_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catalog {
    Tree,
    Game,
}

/// Only these two namespaces are public through the catalog route.
pub fn catalog_namespace(name: &str) -> Option<Catalog> {
    match name {
        "trees" => Some(Catalog::Tree),
        "games" => Some(Catalog::Game),
        _ => None,
    }
}

/// Direct downloads use only chunk keys with a prefix and an object name.
pub fn chunk_path(path: &str) -> bool {
    path.split('/').count() == 3
        && path.starts_with("chunks/")
        && path
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'/' || b.is_ascii_lowercase())
}

pub fn wasm_filename(name: &str) -> bool {
    name.len() <= 64
        && name.ends_with(".wasm")
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeObject {
    Add,
    Same,
    Replace(u128),
    Keep,
}

/// Within a backup merge, the incoming catalog can replace a conflicting local
/// entry. A conflict on any other object keeps the local entry.
pub fn merge_object(local: Option<u128>, incoming: u128, catalog: bool) -> MergeObject {
    match local {
        None => MergeObject::Add,
        Some(local) if local == incoming => MergeObject::Same,
        Some(local) if catalog => MergeObject::Replace(local),
        Some(_) => MergeObject::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_routes_expose_only_their_public_names() {
        assert_eq!(catalog_namespace("trees"), Some(Catalog::Tree));
        assert_eq!(catalog_namespace("games"), Some(Catalog::Game));
        assert_eq!(catalog_namespace("keys"), None);
        assert_eq!(catalog_namespace("chunks"), None);
        assert!(chunk_path("chunks/aa/aabb"));
        assert!(!chunk_path("keys/aa/aabb"));
        assert!(!chunk_path("chunks/aa/../aabb"));
        assert!(wasm_filename("sake-grow.wasm"));
        assert!(!wasm_filename(".private.wasm"));
        assert!(!wasm_filename("sub/module.wasm"));
        assert!(!wasm_filename("sake.js"));
    }

    #[test]
    fn only_the_catalog_can_replace_a_different_local_object_on_merge() {
        assert_eq!(merge_object(None, 2, false), MergeObject::Add);
        assert_eq!(merge_object(Some(2), 2, true), MergeObject::Same);
        assert_eq!(merge_object(Some(1), 2, true), MergeObject::Replace(1));
        assert_eq!(merge_object(Some(1), 2, false), MergeObject::Keep);
    }
}
