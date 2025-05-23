use std::ops::Range;

use file_icons::FileIcons;
use gpui::{App, Entity, SharedString};
use language::Buffer;
use language_model::{LanguageModelRequestMessage, MessageContent};
use project::ProjectPath;
use serde::{Deserialize, Serialize};
use text::{Anchor, BufferId};
use ui::IconName;
use util::post_inc;

use crate::{context_store::buffer_path_log_err, thread::Thread};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct ContextId(pub(crate) usize);

impl ContextId {
    pub fn post_inc(&mut self) -> Self {
        Self(post_inc(&mut self.0))
    }
}

/// Some context attached to a message in a thread.
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub id: ContextId,
    pub name: SharedString,
    pub parent: Option<SharedString>,
    pub tooltip: Option<SharedString>,
    pub icon_path: Option<SharedString>,
    pub kind: ContextKind,
    /// Joining these strings separated by \n yields text for model. Not refreshed by `snapshot`.
    pub text: Box<[SharedString]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    File,
    Directory,
    Symbol,
    FetchedUrl,
    Thread,
}

impl ContextKind {
    pub fn icon(&self) -> IconName {
        match self {
            ContextKind::File => IconName::File,
            ContextKind::Directory => IconName::Folder,
            ContextKind::Symbol => IconName::Code,
            ContextKind::FetchedUrl => IconName::Globe,
            ContextKind::Thread => IconName::MessageBubbles,
        }
    }
}

#[derive(Debug)]
pub enum AssistantContext {
    File(FileContext),
    Directory(DirectoryContext),
    Symbol(SymbolContext),
    FetchedUrl(FetchedUrlContext),
    Thread(ThreadContext),
}

impl AssistantContext {
    pub fn id(&self) -> ContextId {
        match self {
            Self::File(file) => file.id,
            Self::Directory(directory) => directory.snapshot.id,
            Self::Symbol(symbol) => symbol.id,
            Self::FetchedUrl(url) => url.id,
            Self::Thread(thread) => thread.id,
        }
    }
}

#[derive(Debug)]
pub struct FileContext {
    pub id: ContextId,
    pub context_buffer: ContextBuffer,
}

#[derive(Debug)]
pub struct DirectoryContext {
    pub path: ProjectPath,
    pub context_buffers: Vec<ContextBuffer>,
    pub snapshot: ContextSnapshot,
}

#[derive(Debug)]
pub struct SymbolContext {
    pub id: ContextId,
    pub context_symbol: ContextSymbol,
}

#[derive(Debug)]
pub struct FetchedUrlContext {
    pub id: ContextId,
    pub url: SharedString,
    pub text: SharedString,
}

// TODO: Model<Thread> holds onto the thread even if the thread is deleted. Can either handle this
// explicitly or have a WeakModel<Thread> and remove during snapshot.

#[derive(Debug)]
pub struct ThreadContext {
    pub id: ContextId,
    pub thread: Entity<Thread>,
    pub text: SharedString,
}

// TODO: Model<Buffer> holds onto the buffer even if the file is deleted and closed. Should remove
// the context from the message editor in this case.

#[derive(Debug, Clone)]
pub struct ContextBuffer {
    pub id: BufferId,
    pub buffer: Entity<Buffer>,
    pub version: clock::Global,
    pub text: SharedString,
}

#[derive(Debug, Clone)]
pub struct ContextSymbol {
    pub id: ContextSymbolId,
    pub buffer: Entity<Buffer>,
    pub buffer_version: clock::Global,
    /// The range that the symbol encloses, e.g. for function symbol, this will
    /// include not only the signature, but also the body
    pub enclosing_range: Range<Anchor>,
    pub text: SharedString,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextSymbolId {
    pub path: ProjectPath,
    pub name: SharedString,
    pub range: Range<Anchor>,
}

impl AssistantContext {
    pub fn snapshot(&self, cx: &App) -> Option<ContextSnapshot> {
        match &self {
            Self::File(file_context) => file_context.snapshot(cx),
            Self::Directory(directory_context) => Some(directory_context.snapshot()),
            Self::Symbol(symbol_context) => symbol_context.snapshot(cx),
            Self::FetchedUrl(fetched_url_context) => Some(fetched_url_context.snapshot()),
            Self::Thread(thread_context) => Some(thread_context.snapshot(cx)),
        }
    }
}

impl FileContext {
    pub fn snapshot(&self, cx: &App) -> Option<ContextSnapshot> {
        let buffer = self.context_buffer.buffer.read(cx);
        let path = buffer_path_log_err(buffer, cx)?;
        let full_path: SharedString = path.to_string_lossy().into_owned().into();
        let name = match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned().into(),
            None => full_path.clone(),
        };
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned().into());

        let icon_path = FileIcons::get_icon(&path, cx);

        Some(ContextSnapshot {
            id: self.id,
            name,
            parent,
            tooltip: Some(full_path),
            icon_path,
            kind: ContextKind::File,
            text: Box::new([self.context_buffer.text.clone()]),
        })
    }
}

impl DirectoryContext {
    pub fn new(
        id: ContextId,
        project_path: ProjectPath,
        context_buffers: Vec<ContextBuffer>,
    ) -> DirectoryContext {
        let full_path: SharedString = project_path.path.to_string_lossy().into_owned().into();

        let name = match project_path.path.file_name() {
            Some(name) => name.to_string_lossy().into_owned().into(),
            None => full_path.clone(),
        };

        let parent = project_path
            .path
            .parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned().into());

        // TODO: include directory path in text?
        let text = context_buffers
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .into();

        DirectoryContext {
            path: project_path,
            context_buffers,
            snapshot: ContextSnapshot {
                id,
                name,
                parent,
                tooltip: Some(full_path),
                icon_path: None,
                kind: ContextKind::Directory,
                text,
            },
        }
    }

    pub fn snapshot(&self) -> ContextSnapshot {
        self.snapshot.clone()
    }
}

impl SymbolContext {
    pub fn snapshot(&self, cx: &App) -> Option<ContextSnapshot> {
        let buffer = self.context_symbol.buffer.read(cx);
        let name = self.context_symbol.id.name.clone();
        let path = buffer_path_log_err(buffer, cx)?
            .to_string_lossy()
            .into_owned()
            .into();

        Some(ContextSnapshot {
            id: self.id,
            name,
            parent: Some(path),
            tooltip: None,
            icon_path: None,
            kind: ContextKind::Symbol,
            text: Box::new([self.context_symbol.text.clone()]),
        })
    }
}

impl FetchedUrlContext {
    pub fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot {
            id: self.id,
            name: self.url.clone(),
            parent: None,
            tooltip: None,
            icon_path: None,
            kind: ContextKind::FetchedUrl,
            text: Box::new([self.text.clone()]),
        }
    }
}

impl ThreadContext {
    pub fn snapshot(&self, cx: &App) -> ContextSnapshot {
        let thread = self.thread.read(cx);
        ContextSnapshot {
            id: self.id,
            name: thread.summary().unwrap_or("New thread".into()),
            parent: None,
            tooltip: None,
            icon_path: None,
            kind: ContextKind::Thread,
            text: Box::new([self.text.clone()]),
        }
    }
}

pub fn attach_context_to_message(
    message: &mut LanguageModelRequestMessage,
    contexts: impl Iterator<Item = ContextSnapshot>,
) {
    let mut file_context = Vec::new();
    let mut directory_context = Vec::new();
    let mut symbol_context = Vec::new();
    let mut fetch_context = Vec::new();
    let mut thread_context = Vec::new();

    let mut capacity = 0;
    for context in contexts {
        capacity += context.text.len();
        match context.kind {
            ContextKind::File => file_context.push(context),
            ContextKind::Directory => directory_context.push(context),
            ContextKind::Symbol => symbol_context.push(context),
            ContextKind::FetchedUrl => fetch_context.push(context),
            ContextKind::Thread => thread_context.push(context),
        }
    }
    if !file_context.is_empty() {
        capacity += 1;
    }
    if !directory_context.is_empty() {
        capacity += 1;
    }
    if !symbol_context.is_empty() {
        capacity += 1;
    }
    if !fetch_context.is_empty() {
        capacity += 1 + fetch_context.len();
    }
    if !thread_context.is_empty() {
        capacity += 1 + thread_context.len();
    }
    if capacity == 0 {
        return;
    }

    let mut context_chunks = Vec::with_capacity(capacity);

    if !file_context.is_empty() {
        context_chunks.push("The following files are available:\n");
        for context in &file_context {
            for chunk in &context.text {
                context_chunks.push(&chunk);
            }
        }
    }

    if !directory_context.is_empty() {
        context_chunks.push("The following directories are available:\n");
        for context in &directory_context {
            for chunk in &context.text {
                context_chunks.push(&chunk);
            }
        }
    }

    if !symbol_context.is_empty() {
        context_chunks.push("The following symbols are available:\n");
        for context in &symbol_context {
            for chunk in &context.text {
                context_chunks.push(&chunk);
            }
        }
    }

    if !fetch_context.is_empty() {
        context_chunks.push("The following fetched results are available:\n");
        for context in &fetch_context {
            context_chunks.push(&context.name);
            for chunk in &context.text {
                context_chunks.push(&chunk);
            }
        }
    }

    if !thread_context.is_empty() {
        context_chunks.push("The following previous conversation threads are available:\n");
        for context in &thread_context {
            context_chunks.push(&context.name);
            for chunk in &context.text {
                context_chunks.push(&chunk);
            }
        }
    }

    debug_assert!(
        context_chunks.len() == capacity,
        "attach_context_message calculated capacity of {}, but length was {}",
        capacity,
        context_chunks.len()
    );

    if !context_chunks.is_empty() {
        message
            .content
            .push(MessageContent::Text(context_chunks.join("\n")));
    }
}
