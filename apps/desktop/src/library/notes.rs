//! Notes, folders, tags, backlinks, and daily notes on [`Library`].

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::{Date, Note, NoteId, clock};
use ottero_storage::NoteSummary;

use super::{Library, LibraryEvent};

impl Library {
    // ---- Notes -----------------------------------------------------------

    pub fn notes(&self, trashed: bool) -> Result<Vec<NoteSummary>> {
        Ok(self.store.list_notes(trashed)?)
    }

    pub fn search_notes(&self, query: &str, limit: usize) -> Result<Vec<NoteSummary>> {
        Ok(self.store.search_notes(query, limit)?)
    }

    pub fn note(&self, id: NoteId) -> Result<Option<Note>> {
        Ok(self.store.note(id)?)
    }

    pub fn backlinks(&self, title: &str) -> Result<Vec<NoteSummary>> {
        Ok(self.store.backlinks(title)?)
    }

    pub fn find_note_by_title(&self, title: &str) -> Result<Option<NoteId>> {
        Ok(self.store.find_note_by_title(title)?)
    }

    pub fn suggest_note_titles(&self, query: &str, limit: usize) -> Result<Vec<(NoteId, String)>> {
        Ok(self.store.suggest_note_titles(query, limit)?)
    }

    /// The note a `[[link]]` points to, created (empty) if it doesn't exist yet,
    /// like Obsidian. Trashed notes are not link targets.
    pub fn open_or_create_by_title(
        &mut self,
        title: &str,
        cx: &mut Context<Self>,
    ) -> Result<NoteId> {
        match self.store.find_note_by_title(title)? {
            Some(id) => Ok(id),
            None => self.create_note(title, cx),
        }
    }

    pub fn note_tags(&self) -> Vec<(String, usize)> {
        self.store.note_tags().unwrap_or_default()
    }

    pub fn notes_with_tag(&self, tag: &str) -> Result<Vec<NoteSummary>> {
        Ok(self.store.notes_with_tag(tag)?)
    }

    pub fn note_ids_with_tag(&self, tag: &str) -> Vec<NoteId> {
        self.store.note_ids_with_tag(tag).unwrap_or_default()
    }

    pub fn unlinked_mentions(&self, id: NoteId, title: &str) -> Vec<NoteSummary> {
        self.store
            .unlinked_mentions(id, title, 20)
            .unwrap_or_default()
    }

    /// Turns the first plain mention of `title` in note `source` into `[[title]]`.
    pub fn link_mention(
        &mut self,
        source: NoteId,
        title: &str,
        cx: &mut Context<Self>,
    ) -> Result<bool> {
        self.update_note(source, cx, |note| {
            let body = note.body();
            let lower = body.to_lowercase();
            let needle = title.to_lowercase();
            let Some(start) = lower
                .find(&needle)
                .filter(|&i| body.is_char_boundary(i) && body.is_char_boundary(i + needle.len()))
            else {
                return Ok(false);
            };
            let end = start + needle.len();
            let new_body = format!(
                "{}[[{}]]{}",
                &body[..start],
                &body[start..end],
                &body[end..]
            );
            let title = note.title().to_owned();
            Ok(note.edit(&title, &new_body, clock::now())?)
        })
    }

    /// The note titled with `date` (YYYY-MM-DD, like Obsidian), created if needed.
    pub fn open_daily_note(&mut self, date: Date, cx: &mut Context<Self>) -> Result<NoteId> {
        let title = daily_note_title(date);
        if let Some(id) = self.store.find_note_by_title(&title)? {
            return Ok(id);
        }
        let note = Note::new(
            &title,
            format!("## {}\n\n", date.strftime("%A, %B %-d")),
            clock::now(),
        )?;
        self.store
            .save_note(&note)
            .context("Couldn't create today's note")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Notes,
            &note.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncNotePayload::from(&note),
        );
        cx.emit(LibraryEvent::NotesChanged);
        Ok(note.id())
    }

    pub fn daily_note_exists(&self, date: Date) -> bool {
        self.store
            .find_note_by_title(&daily_note_title(date))
            .ok()
            .flatten()
            .is_some()
    }

    /// Writes every active note to `dir` as `Title.md`. Returns how many.
    pub fn export_notes(&self, dir: &std::path::Path) -> Result<usize> {
        let notes = self.store.all_notes()?;
        crate::markdown_io::export(&notes, dir)
    }

    /// Imports `.md` files under `dir`. Notes whose title already exists are
    /// skipped. Returns (imported, skipped).
    pub fn import_notes(
        &mut self,
        dir: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Result<(usize, usize)> {
        let files = crate::markdown_io::read_dir(dir)?;
        let (mut imported, mut skipped) = (0, 0);
        for (title, body) in files {
            if self.store.find_note_by_title(&title)?.is_some() {
                skipped += 1;
                continue;
            }
            match Note::new(&title, body, clock::now()) {
                Ok(note) => {
                    self.store.save_note(&note)?;
                    self.record_sync(
                        ottero_sync_protocol::Collection::Notes,
                        &note.id().to_string(),
                        &["record"],
                        false,
                        &crate::sync::SyncNotePayload::from(&note),
                    );
                    imported += 1;
                }
                Err(_) => skipped += 1,
            }
        }
        if imported > 0 {
            cx.emit(LibraryEvent::NotesChanged);
        }
        Ok((imported, skipped))
    }

    pub fn note_count(&self) -> Result<usize> {
        Ok(self.store.count_notes()?)
    }

    pub fn create_note(&mut self, title: &str, cx: &mut Context<Self>) -> Result<NoteId> {
        let note = Note::new(title, "", clock::now())?;
        self.store
            .save_note(&note)
            .context("Couldn't create the note")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Notes,
            &note.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncNotePayload::from(&note),
        );
        cx.emit(LibraryEvent::NotesChanged);
        Ok(note.id())
    }

    /// Saves an edit. Returns `false` (and writes nothing) if the content is unchanged.
    pub fn edit_note(
        &mut self,
        id: NoteId,
        title: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) -> Result<bool> {
        self.update_note(id, cx, |note| Ok(note.edit(title, body, clock::now())?))
    }

    pub fn set_note_pinned(
        &mut self,
        id: NoteId,
        pinned: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_note(id, cx, |note| {
            note.set_pinned(pinned, clock::now())?;
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn trash_note(&mut self, id: NoteId, cx: &mut Context<Self>) -> Result<()> {
        self.update_note(id, cx, |note| {
            note.trash(clock::now());
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn restore_note(&mut self, id: NoteId, cx: &mut Context<Self>) -> Result<()> {
        self.update_note(id, cx, |note| {
            note.restore(clock::now());
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn delete_note_permanently(&mut self, id: NoteId, cx: &mut Context<Self>) -> Result<()> {
        self.store.delete_note_permanently(id)?;
        self.record_sync(
            ottero_sync_protocol::Collection::Notes,
            &id.to_string(),
            &["record"],
            true,
            &serde_json::json!({}),
        );
        cx.emit(LibraryEvent::NotesChanged);
        Ok(())
    }

    pub fn empty_note_trash(&mut self, cx: &mut Context<Self>) -> Result<usize> {
        let trashed = self.store.list_notes(true)?;
        let n = self.store.empty_note_trash()?;
        for summary in trashed {
            self.record_sync(
                ottero_sync_protocol::Collection::Notes,
                &summary.id.to_string(),
                &["record"],
                true,
                &serde_json::json!({}),
            );
        }
        cx.emit(LibraryEvent::NotesChanged);
        Ok(n)
    }

    fn update_note(
        &mut self,
        id: NoteId,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Note) -> Result<bool>,
    ) -> Result<bool> {
        let mut note = self.store.note(id)?.context("That note no longer exists")?;
        if !change(&mut note)? {
            return Ok(false);
        }
        self.store
            .save_note(&note)
            .context("Couldn't save the note")?;
        self.record_sync(
            ottero_sync_protocol::Collection::Notes,
            &note.id().to_string(),
            &["record"],
            false,
            &crate::sync::SyncNotePayload::from(&note),
        );
        cx.emit(LibraryEvent::NotesChanged);
        Ok(true)
    }
}

#[must_use]
pub fn daily_note_title(date: Date) -> String {
    date.to_string()
}
