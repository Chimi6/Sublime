//! The value tree as an arena: one node vector and one text buffer,
//! children chained by index, strings as spans into the buffer. A node is
//! 32 bytes and a string costs its bytes and nothing else, so a document
//! costs about three bytes of tree per byte of dense input (an owned tree
//! of `String`s and `Vec`s cost ten). Text is never freed or moved, so a
//! span stays valid for the tree's life and subtrees copy by node alone.

use std::collections::HashMap;
use std::io;

use crate::value::{Scalar, ValueSink};

/// A range of the text buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// No node.
pub const NONE: u32 = u32::MAX;

/// The first and last child of a container, or `NONE` when empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Children {
    pub first: u32,
    pub last: u32,
}

impl Children {
    pub const EMPTY: Children = Children {
        first: NONE,
        last: NONE,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Data {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(Span),
    /// A date, time, or datetime as written (TOML's four forms).
    Datetime(Span),
    Array(Children),
    Table(Children),
}

impl Data {
    pub fn is_table(&self) -> bool {
        matches!(self, Data::Table(_))
    }

    pub fn is_container(&self) -> bool {
        matches!(self, Data::Table(_) | Data::Array(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    /// The member key in a table; empty for array items and the root.
    pub key: Span,
    /// The next sibling, or `NONE`.
    pub next: u32,
    pub data: Data,
}

#[derive(Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub text: String,
    /// The document's node, or `NONE` while nothing has been placed.
    pub root: u32,
}

impl Tree {
    pub fn new() -> Tree {
        Tree {
            nodes: Vec::new(),
            text: String::new(),
            root: NONE,
        }
    }

    /// Sized for an input of `bytes`, so neither vector grows from empty.
    pub fn with_capacity(bytes: usize) -> Tree {
        Tree {
            nodes: Vec::with_capacity(bytes / 16),
            text: String::with_capacity(bytes / 2),
            root: NONE,
        }
    }

    pub fn str(&self, span: Span) -> &str {
        &self.text[span.start as usize..span.end as usize]
    }

    pub fn key(&self, id: u32) -> &str {
        self.str(self.nodes[id as usize].key)
    }

    pub fn data(&self, id: u32) -> Data {
        self.nodes[id as usize].data
    }

    pub fn intern(&mut self, text: &str) -> Span {
        let start = self.text.len() as u32;
        self.text.push_str(text);
        Span {
            start,
            end: self.text.len() as u32,
        }
    }

    /// Adds a node with no sibling; the caller links it with `append`.
    pub fn push(&mut self, key: Span, data: Data) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            key,
            next: NONE,
            data,
        });
        id
    }

    /// Adds a table or array with no children yet.
    pub fn push_table(&mut self, key: Span) -> u32 {
        self.push(key, Data::Table(Children::EMPTY))
    }

    pub fn push_array(&mut self, key: Span) -> u32 {
        self.push(key, Data::Array(Children::EMPTY))
    }

    /// Appends `child` to the container `parent`.
    pub fn append(&mut self, parent: u32, child: u32) {
        let last = match &mut self.nodes[parent as usize].data {
            Data::Array(children) | Data::Table(children) => {
                if children.first == NONE {
                    children.first = child;
                    children.last = child;
                    return;
                }
                let last = children.last;
                children.last = child;
                last
            }
            _ => return,
        };
        self.nodes[last as usize].next = child;
    }

    /// The children of a container, first to last.
    pub fn children(&self, parent: u32) -> ChildIter<'_> {
        let first = match self.nodes[parent as usize].data {
            Data::Array(children) | Data::Table(children) => children.first,
            _ => NONE,
        };
        ChildIter {
            tree: self,
            at: first,
        }
    }

    pub fn child_count(&self, parent: u32) -> usize {
        self.children(parent).count()
    }

    /// The member of `parent` with `key`, by a scan of the chain.
    pub fn find_member(&self, parent: u32, key: &str) -> Option<u32> {
        self.children(parent).find(|id| self.key(*id) == key)
    }

    /// Links `node` into `parent`'s chain right after `after`.
    pub fn insert_after(&mut self, parent: u32, after: u32, node: u32) {
        let following = self.nodes[after as usize].next;
        self.nodes[node as usize].next = following;
        self.nodes[after as usize].next = node;
        if let Data::Array(children) | Data::Table(children) = &mut self.nodes[parent as usize].data
        {
            if children.last == after {
                children.last = node;
            }
        }
    }

    /// Takes `node` out of `parent`'s chain.
    pub fn unlink(&mut self, parent: u32, node: u32) {
        let following = self.nodes[node as usize].next;
        let mut previous = NONE;
        let mut at = match self.nodes[parent as usize].data {
            Data::Array(children) | Data::Table(children) => children.first,
            _ => return,
        };
        while at != NONE && at != node {
            previous = at;
            at = self.nodes[at as usize].next;
        }
        if at != node {
            return;
        }
        if previous != NONE {
            self.nodes[previous as usize].next = following;
        }
        if let Data::Array(children) | Data::Table(children) = &mut self.nodes[parent as usize].data
        {
            if children.first == node {
                children.first = following;
            }
            if children.last == node {
                children.last = previous;
            }
        }
        self.nodes[node as usize].next = NONE;
    }

    /// A deep copy of the subtree at `id` (text is shared), unlinked.
    pub fn copy_subtree(&mut self, id: u32) -> u32 {
        let node = self.nodes[id as usize];
        let copy = self.push(node.key, node.data);
        if node.data.is_container() {
            let empty = match node.data {
                Data::Table(_) => Data::Table(Children::EMPTY),
                _ => Data::Array(Children::EMPTY),
            };
            self.nodes[copy as usize].data = empty;
            let mut at = match node.data {
                Data::Array(children) | Data::Table(children) => children.first,
                _ => NONE,
            };
            while at != NONE {
                let child = self.copy_subtree(at);
                self.append(copy, child);
                at = self.nodes[at as usize].next;
            }
        }
        copy
    }
}

pub struct ChildIter<'a> {
    tree: &'a Tree,
    at: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.at == NONE {
            return None;
        }
        let id = self.at;
        self.at = self.tree.nodes[id as usize].next;
        Some(id)
    }
}

/// Members beyond this count get a hash index.
const INDEX_THRESHOLD: usize = 16;

/// Finds members by key in tables under construction: a scan of the
/// chain while a table is small, a lazily built map once it grows, so a
/// document of many small tables allocates no maps and one huge table
/// stays linear. One index serves every table of a tree.
#[derive(Default)]
pub struct MemberIndex {
    big: HashMap<u32, HashMap<String, u32>>,
}

impl MemberIndex {
    pub fn find(&mut self, tree: &Tree, parent: u32, key: &str) -> Option<u32> {
        if let Some(map) = self.big.get(&parent) {
            return map.get(key).copied();
        }
        let mut count = 0;
        let mut found = None;
        for id in tree.children(parent) {
            count += 1;
            if found.is_none() && tree.key(id) == key {
                found = Some(id);
            }
        }
        if count >= INDEX_THRESHOLD {
            let mut map: HashMap<String, u32> = HashMap::with_capacity(count * 2);
            for id in tree.children(parent) {
                map.entry(tree.key(id).to_string()).or_insert(id);
            }
            self.big.insert(parent, map);
        }
        found
    }

    /// Call after appending `id` under `parent`.
    pub fn record(&mut self, tree: &Tree, parent: u32, id: u32) {
        if let Some(map) = self.big.get_mut(&parent) {
            map.entry(tree.key(id).to_string()).or_insert(id);
        }
    }
}

/// Builds a `Tree` from sink calls.
pub struct TreeSink {
    tree: Tree,
    open: Vec<u32>,
    pending_key: Span,
}

impl TreeSink {
    pub fn new(tree: Tree) -> TreeSink {
        TreeSink {
            tree,
            open: Vec::new(),
            pending_key: Span::default(),
        }
    }

    pub fn finish(self) -> Tree {
        self.tree
    }

    fn place(&mut self, data: Data) -> u32 {
        let key = std::mem::take(&mut self.pending_key);
        let id = self.tree.push(key, data);
        match self.open.last() {
            Some(parent) => self.tree.append(*parent, id),
            None => self.tree.root = id,
        }
        id
    }
}

impl ValueSink for TreeSink {
    fn begin_table(&mut self) -> io::Result<()> {
        let id = self.place(Data::Table(Children::EMPTY));
        self.open.push(id);
        Ok(())
    }

    fn key(&mut self, key: &str) -> io::Result<()> {
        self.pending_key = self.tree.intern(key);
        Ok(())
    }

    fn end_table(&mut self) -> io::Result<()> {
        self.open.pop();
        Ok(())
    }

    fn begin_array(&mut self) -> io::Result<()> {
        let id = self.place(Data::Array(Children::EMPTY));
        self.open.push(id);
        Ok(())
    }

    fn end_array(&mut self) -> io::Result<()> {
        self.open.pop();
        Ok(())
    }

    fn scalar(&mut self, scalar: Scalar<'_>) -> io::Result<()> {
        let data = match scalar {
            Scalar::Null => Data::Null,
            Scalar::Bool(flag) => Data::Bool(flag),
            Scalar::Integer(number) => Data::Integer(number),
            Scalar::Float(number) => Data::Float(number),
            Scalar::String(text) => Data::Text(self.tree.intern(text)),
            Scalar::Datetime(text) => Data::Datetime(self.tree.intern(text)),
        };
        self.place(data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_is_small() {
        assert!(
            std::mem::size_of::<Node>() <= 32,
            "{}",
            std::mem::size_of::<Node>()
        );
    }

    #[test]
    fn children_chain_in_order_and_copy_deep() {
        let mut sink = TreeSink::new(Tree::new());
        sink.begin_table().unwrap();
        sink.key("a").unwrap();
        sink.scalar(Scalar::Integer(1)).unwrap();
        sink.key("b").unwrap();
        sink.begin_array().unwrap();
        sink.scalar(Scalar::String("x")).unwrap();
        sink.scalar(Scalar::Null).unwrap();
        sink.end_array().unwrap();
        sink.end_table().unwrap();
        let mut tree = sink.finish();
        let root = tree.root;
        let keys: Vec<&str> = tree.children(root).map(|id| tree.key(id)).collect();
        assert_eq!(keys, vec!["a", "b"]);
        let array = tree.find_member(root, "b").unwrap();
        assert_eq!(tree.child_count(array), 2);
        let copy = tree.copy_subtree(root);
        assert_ne!(copy, root);
        let copied_array = tree.find_member(copy, "b").unwrap();
        assert_ne!(copied_array, array);
        assert_eq!(tree.child_count(copied_array), 2);
    }

    #[test]
    fn member_index_finds_in_small_and_big_tables() {
        let mut tree = Tree::new();
        let table = tree.push_table(Span::default());
        tree.root = table;
        let mut index = MemberIndex::default();
        for number in 0..40 {
            let key = tree.intern(&format!("k{number}"));
            let id = tree.push(key, Data::Integer(number));
            assert!(index.find(&tree, table, &format!("k{number}")).is_none());
            tree.append(table, id);
            index.record(&tree, table, id);
        }
        assert!(index.find(&tree, table, "k39").is_some());
        assert!(index.find(&tree, table, "k40").is_none());
    }
}
