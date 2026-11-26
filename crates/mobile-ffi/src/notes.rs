//! Notes operations exposed to mobile clients.

use ottero_core::{Note, NoteId, clock};
use ottero_storage::{NoteSummary, Store};

use crate::dto::{NoteDetail, NoteListItem, TagCount};
use crate::error::MobileError;

pub(crate) fn summary_to_dto(s: NoteSummary) -> NoteListItem {
    NoteListItem {
        id: s.id.to_string(),
        title: s.title,
        preview: s.preview,
        pinned: s.pinned,
        trashed: false,
        updated_at_ms: s.updated_at.as_millisecond(),
    }
}

fn note_to_detail(n: &Note) -> NoteDetail {
    NoteDetail {
        id: n.id().to_string(),
        title: n.title().to_owned(),
        body: n.body().to_owned(),
        pinned: n.is_pinned(),
        trashed: n.is_trashed(),
        updated_at_ms: n.updated_at().as_millisecond(),
    }
}

fn parse_note_id(id: &str) -> Result<NoteId, MobileError> {
    id.parse::<NoteId>().map_err(|_| MobileError::InvalidInput {
        reason: "Invalid note ID".into(),
    })
}

pub(crate) fn list_notes(
    store: &Store,
    query: Option<String>,
) -> Result<Vec<NoteListItem>, MobileError> {
    let rows = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => store.search_notes(q, 100)?,
        None => store.list_notes(false)?,
    };
    Ok(rows.into_iter().map(summary_to_dto).collect())
}

pub(crate) fn get_note(store: &Store, id: &str) -> Result<NoteDetail, MobileError> {
    let note_id = parse_note_id(id)?;
    let note = store.note(note_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Note not found".into(),
    })?;
    Ok(note_to_detail(&note))
}

pub(crate) fn create_note(
    store: &mut Store,
    title: &str,
    body: &str,
) -> Result<NoteDetail, MobileError> {
    let note = Note::new(title, body, clock::now())?;
    store.save_note(&note)?;
    Ok(note_to_detail(&note))
}

pub(crate) fn update_note(
    store: &mut Store,
    id: &str,
    title: &str,
    body: &str,
) -> Result<NoteDetail, MobileError> {
    let note_id = parse_note_id(id)?;
    let mut note = store.note(note_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Note not found".into(),
    })?;
    if note.edit(title, body, clock::now())? {
        store.save_note(&note)?;
    }
    Ok(note_to_detail(&note))
}

pub(crate) fn set_note_pinned(
    store: &mut Store,
    id: &str,
    pinned: bool,
) -> Result<NoteDetail, MobileError> {
    let note_id = parse_note_id(id)?;
    let mut note = store.note(note_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Note not found".into(),
    })?;
    if note.is_pinned() != pinned {
        note.set_pinned(pinned, clock::now())?;
        store.save_note(&note)?;
    }
    Ok(note_to_detail(&note))
}

pub(crate) fn trash_note(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let note_id = parse_note_id(id)?;
    let mut note = store.note(note_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Note not found".into(),
    })?;
    note.trash(clock::now());
    store.save_note(&note)?;
    Ok(())
}

pub(crate) fn restore_note(store: &mut Store, id: &str) -> Result<NoteDetail, MobileError> {
    let note_id = parse_note_id(id)?;
    let mut note = store.note(note_id)?.ok_or_else(|| MobileError::NotFound {
        reason: "Note not found".into(),
    })?;
    note.restore(clock::now());
    store.save_note(&note)?;
    Ok(note_to_detail(&note))
}

pub(crate) fn list_trashed_notes(store: &Store) -> Result<Vec<NoteListItem>, MobileError> {
    let rows = store.list_notes(true)?;
    Ok(rows
        .into_iter()
        .map(|s| NoteListItem {
            trashed: true,
            ..summary_to_dto(s)
        })
        .collect())
}

pub(crate) fn delete_note_permanently(store: &mut Store, id: &str) -> Result<(), MobileError> {
    let note_id = parse_note_id(id)?;
    store.delete_note_permanently(note_id)?;
    Ok(())
}

pub(crate) fn list_tags(store: &Store) -> Result<Vec<TagCount>, MobileError> {
    let tags = store.note_tags()?;
    Ok(tags
        .into_iter()
        .map(|(name, count)| TagCount {
            name,
            count: u32::try_from(count).unwrap_or(u32::MAX),
        })
        .collect())
}

pub(crate) fn list_notes_by_tag(
    store: &Store,
    tag: &str,
) -> Result<Vec<NoteListItem>, MobileError> {
    let rows = store.notes_with_tag(tag)?;
    Ok(rows.into_iter().map(summary_to_dto).collect())
}
