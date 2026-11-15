//! Branching message store, ported from `message-repository.ts`.
//!
//! Messages form a forest: each has a parent (or none at the root) and an
//! ordered list of children. Each parent remembers its selected child, and the
//! head is the last message of the selected path. `messages()` returns that path
//! from the root, cached until the next change. Messages are `Arc`s, so handing
//! the path to a model adapter does not copy content.

use crate::types::{MessageId, RunConfig, ThreadMessage};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;

/// A stored message and where it attaches, for export and import.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedMessage {
    /// The message.
    pub message: Arc<ThreadMessage>,
    /// Its parent, or `None` for a root message.
    pub parent_id: Option<MessageId>,
    /// The run configuration it was sent with, if recorded.
    pub run_config: Option<RunConfig>,
}

/// Serializable snapshot of a repository (`ExportedMessageRepository`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportedRepository {
    /// `None` imports with the last listed message as head.
    pub head_id: Option<Option<MessageId>>,
    /// Every message, parents before children.
    pub messages: Vec<ExportedMessage>,
}

impl ExportedRepository {
    /// A linear history: each message follows the previous one.
    pub fn from_linear(messages: impl IntoIterator<Item = ThreadMessage>) -> Self {
        let mut parent: Option<MessageId> = None;
        let messages = messages
            .into_iter()
            .map(|message| {
                let id = message.id().clone();
                let item = ExportedMessage {
                    message: Arc::new(message),
                    parent_id: parent.take(),
                    run_config: None,
                };
                parent = Some(id);
                item
            })
            .collect();
        Self {
            head_id: None,
            messages,
        }
    }
}

/// An operation that would corrupt the tree. Upstream reports these as
/// internal-bug exceptions; here the repository is left unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepositoryError {
    /// No message has this id.
    MessageNotFound(MessageId),
    /// No message has the given parent id.
    ParentNotFound(MessageId),
    /// The update would make a message its own ancestor.
    Cycle(MessageId),
    /// The replacement is the deleted message or one of its descendants.
    InvalidReplacement(MessageId),
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageNotFound(id) => write!(f, "message {id} not found"),
            Self::ParentNotFound(id) => write!(f, "parent message {id} not found"),
            Self::Cycle(id) => write!(f, "message {id} would become its own ancestor"),
            Self::InvalidReplacement(id) => {
                write!(f, "{id} cannot replace a message it descends from")
            }
        }
    }
}
impl std::error::Error for RepositoryError {}

/// A message with its position on its own branch.
#[derive(Clone, Debug)]
pub struct MessageEntry {
    /// The message.
    pub message: Arc<ThreadMessage>,
    /// Its parent, or `None` for a root message.
    pub parent_id: Option<MessageId>,
    /// Depth from the root, which is also its index on any path through it.
    pub index: usize,
}

/// Changes to the selected messages since a consumer's last read.
///
/// Each consumer keeps its own version; reading does not consume changes.
/// Versions belong to one repository/runtime and must be discarded on rebind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageChanges {
    /// Pass this back on the next read, including after a full rebuild.
    pub version: u64,
    /// Sorted, unique indices whose contents changed. `None` requires a full
    /// read: the shape changed, the consumer is new, or the bounded journal
    /// no longer covers its version. `Some([])` means no message changed.
    pub indices: Option<Vec<usize>>,
}

// A slow or temporarily hidden view falls back to a full read instead of
// retaining an unbounded stream of updates (or old message contents).
const CHANGE_HISTORY: usize = 64;

#[derive(Default)]
struct Children {
    ids: Vec<MessageId>,
    /// The selected child.
    next: Option<MessageId>,
}

struct Node {
    message: Arc<ThreadMessage>,
    parent: Option<MessageId>,
    children: Children,
    level: usize,
}

/// A tree of messages with one selected path, root to head.
#[derive(Default)]
pub struct MessageRepository {
    nodes: HashMap<MessageId, Node>,
    root: Children,
    head: Option<MessageId>,
    path: RefCell<Option<Arc<[Arc<ThreadMessage>]>>>,
    /// Bumped whenever the tree's shape or the selected path changes.
    shape_version: u64,
    change_version: u64,
    change_base: u64,
    changes: VecDeque<(u64, usize)>,
}

impl MessageRepository {
    /// An empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of messages in the tree, on every branch.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    /// Whether the tree has no messages.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// Whether a message with this id is in the tree.
    pub fn contains(&self, id: &MessageId) -> bool {
        self.nodes.contains_key(id)
    }
    /// The last message of the selected path.
    pub fn head_id(&self) -> Option<&MessageId> {
        self.head.as_ref()
    }

    /// The head after skipping optimistic messages.
    pub fn canonical_head_id(&self) -> Option<&MessageId> {
        let mut head = self.head.as_ref();
        while let Some(node) = head.and_then(|id| self.nodes.get(id)) {
            if !node.message.is_optimistic() {
                break;
            }
            head = node.parent.as_ref();
        }
        head
    }

    /// The selected path from the root to the head.
    pub fn messages(&self) -> Arc<[Arc<ThreadMessage>]> {
        if let Some(path) = &*self.path.borrow() {
            return path.clone();
        }
        let path: Arc<[Arc<ThreadMessage>]> = self.path_to(self.head.as_ref()).into();
        *self.path.borrow_mut() = Some(path.clone());
        path
    }

    /// The path from the root to `head_id`.
    pub fn messages_to(
        &self,
        head_id: &MessageId,
    ) -> Result<Vec<Arc<ThreadMessage>>, RepositoryError> {
        if !self.nodes.contains_key(head_id) {
            return Err(RepositoryError::MessageNotFound(head_id.clone()));
        }
        Ok(self.path_to(Some(head_id)))
    }

    fn path_to(&self, head: Option<&MessageId>) -> Vec<Arc<ThreadMessage>> {
        let mut path = Vec::with_capacity(
            head.and_then(|id| self.nodes.get(id))
                .map_or(0, |n| n.level + 1),
        );
        let mut current = head;
        while let Some(node) = current.and_then(|id| self.nodes.get(id)) {
            path.push(node.message.clone());
            current = node.parent.as_ref();
        }
        path.reverse();
        path
    }

    /// A message with its parent and depth.
    pub fn get(&self, id: &MessageId) -> Result<MessageEntry, RepositoryError> {
        let node = self.node(id)?;
        Ok(MessageEntry {
            message: node.message.clone(),
            parent_id: node.parent.clone(),
            index: node.level,
        })
    }

    /// Whether the message has replies.
    pub fn has_children(&self, id: &MessageId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| !node.children.ids.is_empty())
    }

    /// The ids of the message and its siblings, in creation order.
    pub fn branches(&self, id: &MessageId) -> Result<&[MessageId], RepositoryError> {
        let parent = self.node(id)?.parent.clone();
        Ok(&self.children_of(parent.as_ref()).ids)
    }

    /// `(position, count)` of the message among its siblings, 1-based, without
    /// allocating (what a branch picker shows).
    pub fn branch_position(&self, id: &MessageId) -> Result<(usize, usize), RepositoryError> {
        let node = self.node(id)?;
        let ids = &self.children_of(node.parent.as_ref()).ids;
        let position = ids.iter().position(|b| b == id).map_or(1, |p| p + 1);
        Ok((position, ids.len().max(1)))
    }

    fn node(&self, id: &MessageId) -> Result<&Node, RepositoryError> {
        self.nodes
            .get(id)
            .ok_or_else(|| RepositoryError::MessageNotFound(id.clone()))
    }
    fn children_of(&self, parent: Option<&MessageId>) -> &Children {
        match parent {
            Some(id) => &self.nodes[id].children,
            None => &self.root,
        }
    }
    fn children_of_mut(&mut self, parent: Option<&MessageId>) -> &mut Children {
        match parent {
            Some(id) => &mut self.nodes.get_mut(id).expect("parent is stored").children,
            None => &mut self.root,
        }
    }
    fn dirty(&mut self) {
        *self.path.get_mut() = None;
        self.shape_version = self.shape_version.wrapping_add(1);
        self.reset_changes();
    }

    fn reset_changes(&mut self) {
        self.change_version = self.change_version.wrapping_add(1);
        self.change_base = self.change_version;
        self.changes.clear();
    }

    fn record_change(&mut self, index: usize) {
        self.change_version = self.change_version.wrapping_add(1);
        if self.changes.len() == CHANGE_HISTORY {
            self.change_base = self.changes.pop_front().expect("journal is full").0;
        }
        self.changes.push_back((self.change_version, index));
    }

    /// Selected-message changes since `version`, with bounded bookkeeping.
    /// Call [`messages`](Self::messages) for a full read when `indices` is
    /// `None`. A consumer must also refresh global runtime state (running,
    /// capabilities), which is not part of this repository's journal.
    pub fn message_changes_since(&self, version: Option<u64>) -> MessageChanges {
        let indices = version.and_then(|version| {
            // Wrapping distances preserve the cursor across u64 rollover.
            let behind = self.change_version.wrapping_sub(version);
            if behind > self.change_version.wrapping_sub(self.change_base) {
                return None;
            }
            let mut indices: Vec<_> = self
                .changes
                .iter()
                .rev()
                .take_while(|(changed, _)| self.change_version.wrapping_sub(*changed) < behind)
                .map(|(_, index)| *index)
                .collect();
            indices.sort_unstable();
            indices.dedup();
            Some(indices)
        });
        MessageChanges {
            version: self.change_version,
            indices,
        }
    }

    /// Changes whenever messages are added, moved or removed, or the selected
    /// path changes; not when a message is updated in place (streaming). While
    /// it is unchanged, every message keeps its branch position.
    pub fn shape_version(&self) -> u64 {
        self.shape_version
    }

    /// A message changed in place: patch the cached path (copying pointers)
    /// instead of rebuilding it with a lookup per ancestor. Streaming replaces
    /// the head message on every update, so this keeps an update cheap on a
    /// long thread.
    fn replace_in_path(&mut self, level: usize, id: &MessageId, message: Arc<ThreadMessage>) {
        let Some(path) = self.path.get_mut().as_mut() else {
            // No materialized path to establish membership against. A full
            // read remains necessary, even for a cursor acquired beforehand.
            self.reset_changes();
            return;
        };
        if path.get(level).map(|m| m.id()) == Some(id) {
            if Arc::ptr_eq(&path[level], &message) {
                return;
            }
            if let Some(slots) = Arc::get_mut(path) {
                // No reader holds this path: patch it in place, O(1).
                slots[level] = message;
            } else {
                let mut patched: Vec<Arc<ThreadMessage>> = path.to_vec();
                patched[level] = message;
                *path = patched.into();
            }
            self.record_change(level);
        }
        // Otherwise the message is on another branch: the selected path is unchanged.
    }

    /// Follow selected children from `start` (`None` for the root) to a leaf.
    fn find_head(&self, start: Option<&MessageId>) -> Option<MessageId> {
        let mut head = start.cloned();
        while let Some(next) = self.children_of(head.as_ref()).next.clone() {
            head = Some(next);
        }
        head
    }

    /// The invariant appends rely on: every link from the root to the head is
    /// selected, so following selected children from the root reaches the head.
    fn head_path_selected(&self) -> bool {
        let mut current = self.head.clone();
        while let Some(id) = current {
            let parent = self.nodes[&id].parent.clone();
            if self.children_of(parent.as_ref()).next.as_ref() != Some(&id) {
                return false;
            }
            current = parent;
        }
        true
    }

    /// Select every ancestor link down to `id`.
    fn select_path_to(&mut self, id: &MessageId) {
        let mut current = Some(id.clone());
        while let Some(id) = current {
            let parent = self.nodes[&id].parent.clone();
            self.children_of_mut(parent.as_ref()).next = Some(id);
            current = parent;
        }
    }

    fn update_levels(&mut self, id: &MessageId, level: usize) {
        let mut pending = vec![(id.clone(), level)];
        while let Some((id, level)) = pending.pop() {
            let node = self.nodes.get_mut(&id).expect("descendant is stored");
            node.level = level;
            pending.extend(
                node.children
                    .ids
                    .iter()
                    .map(|child| (child.clone(), level + 1)),
            );
        }
    }

    fn is_ancestor_or_self(&self, candidate: &MessageId, of: Option<&MessageId>) -> bool {
        let mut current = of;
        while let Some(id) = current {
            if id == candidate {
                return true;
            }
            current = self.nodes.get(id).and_then(|n| n.parent.as_ref());
        }
        false
    }

    /// Detach `child` from its parent, selecting the last remaining sibling.
    fn cut(&mut self, child: &MessageId) {
        let parent = self.nodes[child].parent.clone();
        let siblings = self.children_of_mut(parent.as_ref());
        siblings.ids.retain(|id| id != child);
        if siblings.next.as_ref() == Some(child) {
            siblings.next = siblings.ids.last().cloned();
        }
    }

    /// Attach `child` (already stored) as the last child of `parent`.
    fn link(&mut self, parent: Option<&MessageId>, child: &MessageId) {
        let head_was_parent = self.head.as_ref() == parent && parent.is_some();
        self.children_of_mut(parent).ids.push(child.clone());
        self.nodes.get_mut(child).expect("child is stored").parent = parent.cloned();
        if self.find_head(Some(child)) == self.head && self.head.is_some() {
            self.select_path_to(child);
        } else if self.children_of(parent).next.is_none() {
            self.children_of_mut(parent).next = Some(child.clone());
            if head_was_parent {
                self.head = self.find_head(Some(child));
            }
        }
        let level = parent.map_or(0, |p| self.nodes[p].level + 1);
        self.update_levels(child, level);
    }

    /// Insert a new message after `parent_id`, or replace the stored message
    /// with the same id, moving it under `parent_id` if that changed.
    pub fn add_or_update(
        &mut self,
        parent_id: Option<&MessageId>,
        message: impl Into<Arc<ThreadMessage>>,
    ) -> Result<(), RepositoryError> {
        let message = message.into();
        let id = message.id().clone();
        if let Some(parent) = parent_id {
            if !self.nodes.contains_key(parent) {
                return Err(RepositoryError::ParentNotFound(parent.clone()));
            }
        }
        if let Some(existing) = self.nodes.get(&id) {
            if existing.parent.as_ref() != parent_id {
                if self.is_ancestor_or_self(&id, parent_id) {
                    return Err(RepositoryError::Cycle(id));
                }
                self.cut(&id);
                self.link(parent_id, &id);
                // The tree changed shape, so the selected path may too.
                self.dirty();
            }
            let node = self.nodes.get_mut(&id).expect("checked above");
            let level = node.level;
            node.message = message.clone();
            self.replace_in_path(level, &id, message);
            return Ok(());
        }
        let level = parent_id.map_or(0, |p| self.nodes[p].level + 1);
        self.nodes.insert(
            id.clone(),
            Node {
                message,
                parent: parent_id.cloned(),
                children: Children::default(),
                level,
            },
        );
        let head_was_parent = self.head.as_ref() == parent_id;
        self.link(parent_id, &id);
        if head_was_parent {
            // Every link above the head is selected (see `head_path_selected`),
            // so appending to the head selects one more link. Walking to the
            // root here made importing or growing a long thread quadratic.
            self.head = Some(id.clone());
            self.children_of_mut(parent_id).next = Some(id.clone());
        }
        debug_assert!(self.head_path_selected());
        self.dirty();
        Ok(())
    }

    /// Remove a message. Its children move to `replacement`: by default its
    /// parent, `Some(None)` for the root, or `Some(Some(id))` for another message.
    pub fn delete(
        &mut self,
        id: &MessageId,
        replacement: Option<Option<&MessageId>>,
    ) -> Result<(), RepositoryError> {
        let parent = self.node(id)?.parent.clone();
        let replacement: Option<MessageId> = match replacement {
            None => parent,
            Some(None) => None,
            Some(Some(other)) => {
                self.node(other)?;
                Some(other.clone())
            }
        };
        if self.is_ancestor_or_self(id, replacement.as_ref()) {
            return Err(RepositoryError::InvalidReplacement(id.clone()));
        }
        for child in self.nodes[id].children.ids.clone() {
            self.cut(&child);
            self.link(replacement.as_ref(), &child);
        }
        self.cut(id);
        self.nodes.remove(id);
        if self.head.as_ref() == Some(id) {
            self.head = self.find_head(replacement.as_ref());
            if let Some(head) = self.head.clone() {
                // The replacement may sit on an unselected branch.
                self.select_path_to(&head);
            }
        }
        self.dirty();
        Ok(())
    }

    /// Make the branch through `id` current, selecting its last-used descendants.
    pub fn switch_to_branch(&mut self, id: &MessageId) -> Result<(), RepositoryError> {
        self.node(id)?;
        let previous = self.head.clone();
        self.select_path_to(id);
        self.head = self.find_head(Some(id));
        self.evict_off_branch_optimistic(previous);
        self.dirty();
        Ok(())
    }

    /// Make `id` the head and delete everything below it; `None` clears all.
    pub fn reset_head(&mut self, id: Option<&MessageId>) -> Result<(), RepositoryError> {
        self.set_head(id, true)
    }

    fn set_head(&mut self, id: Option<&MessageId>, prune: bool) -> Result<(), RepositoryError> {
        let Some(id) = id else {
            self.clear();
            return Ok(());
        };
        self.node(id)?;
        let previous = self.head.clone();
        if prune {
            let mut pending =
                std::mem::take(&mut self.nodes.get_mut(id).expect("checked").children).ids;
            while let Some(child) = pending.pop() {
                if let Some(node) = self.nodes.remove(&child) {
                    pending.extend(node.children.ids);
                }
            }
        }
        self.head = Some(id.clone());
        self.select_path_to(id);
        self.evict_off_branch_optimistic(previous);
        self.dirty();
        Ok(())
    }

    /// Drop optimistic messages on the branch the head just left.
    fn evict_off_branch_optimistic(&mut self, previous: Option<MessageId>) {
        let on_head: HashSet<MessageId> = self
            .path_to(self.head.as_ref())
            .iter()
            .map(|m| m.id().clone())
            .collect();
        let mut stale = Vec::new();
        let mut current = previous;
        while let Some(id) = current {
            if on_head.contains(&id) {
                break;
            }
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            if node.message.is_optimistic() {
                stale.push(id.clone());
            }
            current = node.parent.clone();
        }
        for id in stale {
            if self.nodes.contains_key(&id) {
                let _ = self.delete(&id, None);
            }
        }
    }

    /// Remove every message.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = Children::default();
        self.head = None;
        self.dirty();
    }

    /// Persisted messages in parent-first order; optimistic ones are skipped
    /// and their children reattached to the nearest persisted ancestor.
    pub fn export(&self) -> ExportedRepository {
        let mut messages = Vec::with_capacity(self.nodes.len());
        let mut pending: Vec<MessageId> = self.root.ids.iter().rev().cloned().collect();
        while let Some(id) = pending.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            pending.extend(node.children.ids.iter().rev().cloned());
            if node.message.is_optimistic() {
                continue;
            }
            let mut parent = node.parent.as_ref();
            while let Some(p) = parent
                .and_then(|p| self.nodes.get(p))
                .filter(|n| n.message.is_optimistic())
            {
                parent = p.parent.as_ref();
            }
            messages.push(ExportedMessage {
                message: node.message.clone(),
                parent_id: parent.cloned(),
                run_config: None,
            });
        }
        ExportedRepository {
            head_id: Some(self.exported_head_id()),
            messages,
        }
    }

    fn exported_head_id(&self) -> Option<MessageId> {
        let mut head = self.head.clone();
        while let Some(node) = head
            .as_ref()
            .and_then(|id| self.nodes.get(id))
            .filter(|n| n.message.is_optimistic())
        {
            head = node.parent.clone();
        }
        if head == self.head {
            return head;
        }
        while let Some(next) = self.exported_child(head.as_ref()) {
            head = Some(next);
        }
        head
    }

    /// The persisted child to follow below `parent`: the selected one (or its
    /// first persisted descendant), else the nearest persisted sibling.
    fn exported_child(&self, parent: Option<&MessageId>) -> Option<MessageId> {
        let children = self.children_of(parent);
        if let Some(selected) = &children.next {
            if !self.nodes[selected].message.is_optimistic() {
                return Some(selected.clone());
            }
            if let Some(found) = self.exported_child(Some(selected)) {
                return Some(found);
            }
        }
        let selected_index = children
            .next
            .as_ref()
            .and_then(|s| children.ids.iter().position(|id| id == s))
            .unwrap_or(children.ids.len());
        let before = children.ids[..selected_index].iter().rev();
        let after = children
            .ids
            .get(selected_index + 1..)
            .unwrap_or_default()
            .iter()
            .rev();
        for id in before.chain(after) {
            let Some(child) = self.nodes.get(id) else {
                continue;
            };
            if !child.message.is_optimistic() {
                return Some(id.clone());
            }
            if let Some(found) = self.exported_child(Some(id)) {
                return Some(found);
            }
        }
        None
    }

    /// Add or update every listed message (parents first), then select the
    /// listed head, or the last listed message.
    pub fn import(&mut self, exported: ExportedRepository) -> Result<(), RepositoryError> {
        let ordered = with_parents_first(exported.messages, |id| self.nodes.contains_key(id));
        let last = ordered.last().map(|item| item.message.id().clone());
        for item in ordered {
            self.add_or_update(item.parent_id.as_ref(), item.message)?;
        }
        let head = match exported.head_id {
            Some(head) => head,
            None => last,
        };
        self.set_head(head.as_ref(), false)
    }
}

/// Reorder so each listed parent precedes its children (`withParentsFirst`).
pub fn with_parents_first(
    messages: Vec<ExportedMessage>,
    is_stored: impl Fn(&MessageId) -> bool,
) -> Vec<ExportedMessage> {
    let listed: HashSet<MessageId> = messages.iter().map(|m| m.message.id().clone()).collect();
    let mut added: HashSet<MessageId> = HashSet::new();
    let mut waiting: HashMap<MessageId, Vec<ExportedMessage>> = HashMap::new();
    let mut waiting_order: Vec<MessageId> = Vec::new();
    let mut ordered: Vec<ExportedMessage> = Vec::with_capacity(messages.len());
    for item in messages {
        if let Some(parent) = &item.parent_id {
            if listed.contains(parent) && !added.contains(parent) && !is_stored(parent) {
                if !waiting.contains_key(parent) {
                    waiting_order.push(parent.clone());
                }
                waiting.entry(parent.clone()).or_default().push(item);
                continue;
            }
        }
        let mut index = ordered.len();
        ordered.push(item);
        while index < ordered.len() {
            let id = ordered[index].message.id().clone();
            index += 1;
            if !added.insert(id.clone()) {
                continue;
            }
            if let Some(children) = waiting.remove(&id) {
                ordered.extend(children);
            }
        }
    }
    for parent in waiting_order {
        if let Some(children) = waiting.remove(&parent) {
            ordered.extend(children);
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageMetadata, TextPart, Timestamp, UserMessage, UserMessagePart};

    fn msg(id: &str) -> ThreadMessage {
        ThreadMessage::User(UserMessage {
            id: id.into(),
            created_at: Timestamp(0),
            content: vec![UserMessagePart::Text(TextPart::new(id))],
            attachments: vec![],
            metadata: MessageMetadata::default(),
        })
    }
    fn optimistic(id: &str) -> ThreadMessage {
        let mut message = msg(id);
        if let ThreadMessage::User(m) = &mut message {
            m.metadata.is_optimistic = true;
        }
        message
    }
    fn id(s: &str) -> MessageId {
        s.into()
    }
    fn ids(repo: &MessageRepository) -> Vec<String> {
        repo.messages().iter().map(|m| m.id().0.clone()).collect()
    }
    fn chain(repo: &mut MessageRepository, items: &[(&str, Option<&str>)]) {
        for (child, parent) in items {
            repo.add_or_update(parent.map(id).as_ref(), msg(child))
                .unwrap();
        }
    }

    #[test]
    fn change_cursors_cover_multiple_updates_without_consuming_other_readers() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        let before = repo.messages();
        let first = repo.message_changes_since(None);
        assert_eq!(first.indices, None);
        assert_eq!(
            repo.message_changes_since(Some(first.version)).indices,
            Some(vec![])
        );

        repo.add_or_update(Some(&id("b")), msg("c")).unwrap();
        let fast = repo.message_changes_since(Some(first.version));
        assert_eq!(fast.indices, Some(vec![2]));
        repo.add_or_update(None, msg("a")).unwrap();
        repo.add_or_update(Some(&id("b")), msg("c")).unwrap();
        assert_eq!(
            repo.message_changes_since(Some(first.version)).indices,
            Some(vec![0, 2])
        );
        assert_eq!(
            repo.message_changes_since(Some(fast.version)).indices,
            Some(vec![0, 2])
        );
        assert!(!Arc::ptr_eq(&before[0], &repo.messages()[0]));
        assert!(Arc::ptr_eq(&before[1], &repo.messages()[1]));
    }

    #[test]
    fn streaming_updates_keep_retained_snapshots_immutable() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        let _ = repo.messages();
        for _ in 0..5 {
            repo.add_or_update(Some(&id("b")), msg("c")).unwrap();
        }
        // A retained snapshot must not be mutated by later updates.
        let snap = repo.messages();
        let before = snap[2].clone();
        repo.add_or_update(Some(&id("b")), msg("c")).unwrap();
        assert!(Arc::ptr_eq(&snap[2], &before));
        assert!(!Arc::ptr_eq(&snap[2], &repo.messages()[2]));
    }

    #[test]
    fn change_cursors_ignore_unselected_content_but_reset_for_structural_changes() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("a"))],
        );
        let _ = repo.messages();
        let before = repo.message_changes_since(None).version;
        repo.add_or_update(Some(&id("a")), msg("c")).unwrap();
        assert_eq!(
            repo.message_changes_since(Some(before)).indices,
            Some(vec![])
        );

        repo.switch_to_branch(&id("c")).unwrap();
        assert_eq!(repo.message_changes_since(Some(before)).indices, None);
        let _ = repo.messages();
        let switched = repo.message_changes_since(None).version;
        repo.add_or_update(Some(&id("a")), msg("c")).unwrap();
        assert_eq!(
            repo.message_changes_since(Some(switched)).indices,
            Some(vec![1])
        );
        repo.delete(&id("c"), None).unwrap();
        assert_eq!(repo.message_changes_since(Some(switched)).indices, None);
        let deleted = repo.message_changes_since(None).version;
        repo.clear();
        assert_eq!(repo.message_changes_since(Some(deleted)).indices, None);
    }

    #[test]
    fn change_cursors_fall_back_when_the_bounded_history_is_exceeded() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None)]);
        let _ = repo.messages();
        let before = repo.message_changes_since(None).version;
        for _ in 0..CHANGE_HISTORY {
            repo.add_or_update(None, msg("a")).unwrap();
        }
        assert_eq!(
            repo.message_changes_since(Some(before)).indices,
            Some(vec![0])
        );
        let recent = repo.message_changes_since(None).version;
        repo.add_or_update(None, msg("a")).unwrap();
        assert_eq!(repo.message_changes_since(Some(before)).indices, None);
        assert_eq!(
            repo.message_changes_since(Some(recent)).indices,
            Some(vec![0])
        );
        assert_eq!(repo.changes.len(), CHANGE_HISTORY);
        assert_eq!(repo.message_changes_since(Some(recent + 10)).indices, None);
    }

    #[test]
    fn change_cursors_cover_rollover_and_updates_before_a_path_is_materialized() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None)]);
        let before_read = repo.message_changes_since(None).version;
        repo.add_or_update(None, msg("a")).unwrap();
        assert_eq!(repo.message_changes_since(Some(before_read)).indices, None);
        let _ = repo.messages();
        repo.change_version = u64::MAX - 1;
        repo.change_base = u64::MAX - 1;
        repo.changes.clear();
        repo.add_or_update(None, msg("a")).unwrap();
        repo.add_or_update(None, msg("a")).unwrap();
        let rolled = repo.message_changes_since(Some(u64::MAX - 1));
        assert_eq!(rolled.version, 0);
        assert_eq!(rolled.indices, Some(vec![0]));
        assert_eq!(repo.message_changes_since(Some(0)).indices, Some(vec![]));
        repo.clear();
        assert_eq!(repo.message_changes_since(Some(0)).indices, None);
    }

    #[test]
    fn appending_moves_the_head_along_the_selected_path() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        assert_eq!(ids(&repo), ["a", "b", "c"]);
        assert_eq!(repo.get(&id("c")).unwrap().index, 2);
    }

    #[test]
    fn a_sibling_is_a_new_branch_and_switching_restores_descendants() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        repo.add_or_update(Some(&id("a")), msg("b2")).unwrap();
        // Adding off the head does not move it.
        assert_eq!(ids(&repo), ["a", "b", "c"]);
        assert_eq!(repo.branches(&id("b")).unwrap(), [id("b"), id("b2")]);
        repo.switch_to_branch(&id("b2")).unwrap();
        assert_eq!(ids(&repo), ["a", "b2"]);
        repo.switch_to_branch(&id("b")).unwrap();
        assert_eq!(
            ids(&repo),
            ["a", "b", "c"],
            "returning restores the last selected descendant"
        );
    }

    /// The cached selected path must always equal a fresh walk from the head.
    fn assert_path_is_fresh(repo: &MessageRepository) {
        let cached = ids(repo);
        let fresh: Vec<String> = match repo.head_id() {
            Some(head) => repo
                .messages_to(head)
                .unwrap()
                .iter()
                .map(|m| m.id().0.clone())
                .collect(),
            None => Vec::new(),
        };
        assert_eq!(cached, fresh);
    }

    #[test]
    fn the_cached_path_stays_correct_through_updates_and_moves() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[
                ("a", None),
                ("b", Some("a")),
                ("c", Some("b")),
                ("d", Some("a")),
            ],
        );
        assert_eq!(ids(&repo), ["a", "b", "c"], "primes the cache");
        // Updating a message on the path patches it in place.
        let mut edited = msg("b");
        if let ThreadMessage::User(m) = &mut edited {
            m.content = vec![UserMessagePart::Text(TextPart::new("edited"))];
        }
        repo.add_or_update(Some(&id("a")), edited).unwrap();
        assert_eq!(repo.messages()[1].text(), "edited");
        assert_path_is_fresh(&repo);
        // Updating a message on another branch leaves the path alone.
        repo.add_or_update(Some(&id("a")), msg("d")).unwrap();
        assert_eq!(ids(&repo), ["a", "b", "c"]);
        assert_path_is_fresh(&repo);
        // Moving a message under a different parent changes the tree: the path
        // must be rebuilt, not patched.
        repo.add_or_update(Some(&id("d")), msg("c")).unwrap();
        assert_path_is_fresh(&repo);
        repo.add_or_update(Some(&id("a")), msg("c")).unwrap();
        assert_path_is_fresh(&repo);
    }

    #[test]
    fn update_keeps_position_and_replaces_content() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None), ("b", Some("a"))]);
        let mut updated = msg("b");
        if let ThreadMessage::User(m) = &mut updated {
            m.content = vec![UserMessagePart::Text(TextPart::new("edited"))];
        }
        repo.add_or_update(Some(&id("a")), updated).unwrap();
        assert_eq!(repo.messages()[1].text(), "edited");
    }

    #[test]
    fn keeps_the_old_content_when_an_update_is_rejected_as_a_cycle() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None), ("b", Some("a"))]);
        let mut moved = msg("a");
        if let ThreadMessage::User(m) = &mut moved {
            m.content = vec![UserMessagePart::Text(TextPart::new("new"))];
        }
        assert_eq!(
            repo.add_or_update(Some(&id("b")), moved),
            Err(RepositoryError::Cycle(id("a")))
        );
        assert_eq!(repo.messages()[0].text(), "a");
        assert_eq!(ids(&repo), ["a", "b"]);
    }

    #[test]
    fn moves_no_child_when_the_replacement_is_a_descendant_of_the_deleted_message() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        assert_eq!(
            repo.delete(&id("a"), Some(Some(&id("c")))),
            Err(RepositoryError::InvalidReplacement(id("a")))
        );
        assert_eq!(ids(&repo), ["a", "b", "c"]);
        assert_eq!(
            repo.delete(&id("b"), Some(Some(&id("b")))),
            Err(RepositoryError::InvalidReplacement(id("b")))
        );
    }

    #[test]
    fn deleting_reattaches_children_to_the_parent() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        repo.delete(&id("b"), None).unwrap();
        assert_eq!(ids(&repo), ["a", "c"]);
        assert_eq!(repo.get(&id("c")).unwrap().index, 1);
        repo.delete(&id("c"), None).unwrap();
        assert_eq!(
            ids(&repo),
            ["a"],
            "the head falls back to the replacement's selected leaf"
        );
    }

    #[test]
    fn deleting_the_selected_sibling_selects_the_last_remaining_one() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[
                ("a", None),
                ("b1", Some("a")),
                ("b2", Some("a")),
                ("b3", Some("a")),
            ],
        );
        repo.switch_to_branch(&id("b2")).unwrap();
        repo.delete(&id("b2"), None).unwrap();
        assert_eq!(ids(&repo), ["a", "b3"]);
    }

    #[test]
    fn reset_head_prunes_descendants() {
        let mut repo = MessageRepository::new();
        chain(
            &mut repo,
            &[("a", None), ("b", Some("a")), ("c", Some("b"))],
        );
        repo.reset_head(Some(&id("a"))).unwrap();
        assert_eq!(ids(&repo), ["a"]);
        assert!(!repo.contains(&id("b")) && !repo.contains(&id("c")));
        repo.reset_head(None).unwrap();
        assert!(repo.is_empty() && repo.messages().is_empty());
    }

    #[test]
    fn switching_away_evicts_optimistic_messages_on_the_left_branch() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None)]);
        repo.add_or_update(Some(&id("a")), optimistic("tmp"))
            .unwrap();
        repo.add_or_update(Some(&id("a")), msg("server")).unwrap();
        assert_eq!(ids(&repo), ["a", "tmp"]);
        repo.switch_to_branch(&id("server")).unwrap();
        assert!(!repo.contains(&id("tmp")));
        assert_eq!(repo.branches(&id("server")).unwrap(), [id("server")]);
    }

    #[test]
    fn imports_a_message_listed_before_its_parent() {
        let mut repo = MessageRepository::new();
        let exported = ExportedRepository {
            head_id: None,
            messages: vec![
                ExportedMessage {
                    message: Arc::new(msg("b")),
                    parent_id: Some(id("a")),
                    run_config: None,
                },
                ExportedMessage {
                    message: Arc::new(msg("a")),
                    parent_id: None,
                    run_config: None,
                },
            ],
        };
        repo.import(exported).unwrap();
        assert_eq!(
            ids(&repo),
            ["a", "b"],
            "head is the last message in parent-first order"
        );
    }

    fn branchable(items: &[(&str, Option<&str>)], head: &str) -> ExportedRepository {
        ExportedRepository {
            head_id: Some(Some(id(head))),
            messages: items
                .iter()
                .map(|(m, p)| ExportedMessage {
                    message: Arc::new(msg(m)),
                    parent_id: p.map(id),
                    run_config: None,
                })
                .collect(),
        }
    }
    fn exported_ids(repo: &MessageRepository) -> Vec<String> {
        repo.export()
            .messages
            .iter()
            .map(|m| m.message.id().0.clone())
            .collect()
    }

    #[test]
    fn keeps_descendants_when_importing_with_an_inner_head() {
        let items = [
            ("u1", None),
            ("a1", Some("u1")),
            ("u2", Some("a1")),
            ("a2", Some("u2")),
            ("a1b", Some("u1")),
        ];
        for (head, visible) in [("a1", vec!["u1", "a1"]), ("u1", vec!["u1"])] {
            let mut repo = MessageRepository::new();
            repo.import(branchable(&items, head)).unwrap();
            assert_eq!(ids(&repo), visible);
            assert_eq!(repo.head_id(), Some(&id(head)));
            assert_eq!(repo.export().head_id, Some(Some(id(head))));
            assert_eq!(exported_ids(&repo), ["u1", "a1", "u2", "a2", "a1b"]);
        }
    }

    #[test]
    fn appends_a_sibling_from_an_imported_head_with_children() {
        let mut repo = MessageRepository::new();
        repo.import(branchable(&[("u2", None), ("a2", Some("u2"))], "u2"))
            .unwrap();
        repo.add_or_update(Some(&id("u2")), msg("a2b")).unwrap();
        assert_eq!(repo.head_id(), Some(&id("a2b")));
        assert_eq!(ids(&repo), ["u2", "a2b"]);
        assert_eq!(repo.branches(&id("a2b")).unwrap(), [id("a2"), id("a2b")]);
        assert_eq!(exported_ids(&repo), ["u2", "a2", "a2b"]);
        repo.switch_to_branch(&id("u2")).unwrap();
        assert_eq!(repo.head_id(), Some(&id("a2b")));
    }

    #[test]
    fn export_skips_optimistic_messages_and_round_trips() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None), ("b", Some("a"))]);
        repo.add_or_update(Some(&id("b")), optimistic("o")).unwrap();
        assert_eq!(ids(&repo), ["a", "b", "o"]);
        let exported = repo.export();
        assert_eq!(exported.head_id, Some(Some(id("b"))));
        assert_eq!(
            exported
                .messages
                .iter()
                .map(|m| m.message.id().0.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let mut restored = MessageRepository::new();
        restored.import(exported).unwrap();
        assert_eq!(ids(&restored), ["a", "b"]);
    }

    #[test]
    fn export_follows_a_persisted_sibling_when_the_selected_branch_is_optimistic() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None), ("b1", Some("a"))]);
        repo.add_or_update(Some(&id("a")), optimistic("b2"))
            .unwrap();
        repo.switch_to_branch(&id("b2")).unwrap();
        assert_eq!(repo.export().head_id, Some(Some(id("b1"))));
    }

    #[test]
    fn the_path_is_cached_until_the_next_change() {
        let mut repo = MessageRepository::new();
        chain(&mut repo, &[("a", None)]);
        let first = repo.messages();
        assert!(Arc::ptr_eq(&first, &repo.messages()));
        chain(&mut repo, &[("b", Some("a"))]);
        assert!(!Arc::ptr_eq(&first, &repo.messages()));
    }

    #[test]
    fn errors_name_the_missing_message() {
        let mut repo = MessageRepository::new();
        assert_eq!(
            repo.add_or_update(Some(&id("x")), msg("a")),
            Err(RepositoryError::ParentNotFound(id("x")))
        );
        assert_eq!(
            repo.switch_to_branch(&id("x")),
            Err(RepositoryError::MessageNotFound(id("x")))
        );
        assert!(repo.is_empty());
    }
}
