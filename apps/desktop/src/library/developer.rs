//! Developer workspace methods on [`Library`].

use anyhow::{Context as _, Result};
use gpui_kit::Context;

use ottero_core::{Note, NoteId, Project, ProjectId, VaultItemId, clock};
use ottero_crypto::Zeroizing;

use super::{Library, LibraryEvent};

impl Library {
    // ---- Developer workspaces ---------------------------------------------
    pub fn developer_projects(
        &self,
        archived: bool,
    ) -> Result<Vec<ottero_core::developer::DeveloperProject>> {
        Ok(self.store.developer_projects(archived)?)
    }
    pub fn developer_candidates(&self) -> Result<Vec<Project>> {
        let used: std::collections::HashSet<_> = self
            .store
            .developer_projects(false)?
            .into_iter()
            .chain(self.store.developer_projects(true)?)
            .map(|p| p.id())
            .collect();
        Ok(self
            .store
            .projects()?
            .into_iter()
            .map(|(p, _)| p)
            .filter(|p| !used.contains(&p.id()))
            .collect())
    }
    pub fn developer_project(
        &self,
        id: ProjectId,
    ) -> Result<Option<ottero_core::developer::DeveloperProject>> {
        Ok(self.store.developer_project(id)?)
    }
    pub fn create_developer_project(
        &mut self,
        name: &str,
        existing: Option<ProjectId>,
        cx: &mut Context<Self>,
    ) -> Result<ProjectId> {
        let now = clock::now();
        let project = match existing {
            Some(id) => self
                .store
                .project(id)?
                .context("That project no longer exists")?,
            None => Project::new(name, now)?,
        };
        if self.store.developer_project(project.id())?.is_some() {
            anyhow::bail!("This project already has a developer workspace.");
        }
        let mut workspace = ottero_core::developer::DeveloperProject::new(project);
        let title = setup_note_title(workspace.project().name());
        let note = Note::new(&title, "", now)?;
        workspace.set_setup_note(note.id(), now);
        self.store.save_developer_project(&workspace, Some(&note))?;
        cx.emit(LibraryEvent::TasksChanged);
        cx.emit(LibraryEvent::NotesChanged);
        cx.emit(LibraryEvent::DeveloperChanged);
        Ok(workspace.id())
    }
    pub fn update_developer_project(
        &mut self,
        id: ProjectId,
        name: &str,
        links: Vec<ottero_core::developer::ProjectLink>,
        folder: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let now = clock::now();
        let mut w = self
            .store
            .developer_project(id)?
            .context("That developer project no longer exists")?;
        w.update(links, folder, now)?;
        // Keep the task project's name and the developer workspace in one transaction.
        let mut p = w.project().clone();
        p.rename(name, now)?;
        let w = ottero_core::developer::DeveloperProject::from_parts(
            p,
            w.links().to_vec(),
            w.folder().to_owned(),
            w.setup_note(),
            now.max(w.updated_at()),
            w.archived_at(),
        )?;
        self.store.save_developer_project(&w, None)?;
        cx.emit(LibraryEvent::TasksChanged);
        cx.emit(LibraryEvent::DeveloperChanged);
        Ok(())
    }
    pub fn archive_developer_project(
        &mut self,
        id: ProjectId,
        archived: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut w = self
            .store
            .developer_project(id)?
            .context("That developer project no longer exists")?;
        w.set_archived(archived, clock::now());
        self.store.save_developer_project(&w, None)?;
        cx.emit(LibraryEvent::DeveloperChanged);
        Ok(())
    }
    pub fn developer_setup_note(
        &mut self,
        id: ProjectId,
        cx: &mut Context<Self>,
    ) -> Result<NoteId> {
        let mut w = self
            .store
            .developer_project(id)?
            .context("That developer project no longer exists")?;
        if let Some(id) = w.setup_note()
            && let Some(note) = self.store.note(id)?
        {
            if note.is_trashed() {
                anyhow::bail!("Restore the setup note from Notes → Trash first.");
            }
            return Ok(id);
        }
        let note = Note::new(&setup_note_title(w.project().name()), "", clock::now())?;
        w.set_setup_note(note.id(), clock::now());
        self.store.save_developer_project(&w, Some(&note))?;
        cx.emit(LibraryEvent::DeveloperChanged);
        cx.emit(LibraryEvent::NotesChanged);
        Ok(note.id())
    }
    /// Bounded file import. Returns zeroizing text; errors never include bytes or paths.
    pub fn read_environment_file(
        path: &std::path::Path,
    ) -> Result<ottero_crypto::Zeroizing<String>> {
        use std::io::Read as _;
        let file = std::fs::File::open(path)
            .map_err(|_| anyhow::anyhow!("Couldn't open that environment file."))?;
        let mut bytes = ottero_crypto::Zeroizing::new(Vec::new());
        file.take((ottero_core::vault::environment::MAX_ENV_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow::anyhow!("Couldn't read that environment file."))?;
        if bytes.len() > ottero_core::vault::environment::MAX_ENV_BYTES {
            anyhow::bail!("This environment file exceeds the 1 MiB limit.");
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("Choose a UTF-8 environment file."))?;
        Ok(ottero_crypto::Zeroizing::new(text.to_owned()))
    }
    /// Deliberate plaintext export, called only after the native path prompt.
    /// Refuses to overwrite; a private temporary file is persisted atomically.
    pub fn export_environment(&self, id: VaultItemId, path: &std::path::Path) -> Result<()> {
        let text = self.environment_export_text(id)?;
        Self::write_environment_file(path, &text)
    }

    /// Snapshot only after confirmation and path selection, while still unlocked.
    pub fn environment_export_text(&self, id: VaultItemId) -> Result<Zeroizing<String>> {
        if !self.vault_is_unlocked() {
            anyhow::bail!("Unlock Passwords before exporting.");
        }
        let item = self
            .vault_item(id)
            .filter(|i| !i.is_trashed())
            .context("That vault item no longer exists")?;
        let env = item
            .fields()
            .environment
            .as_ref()
            .context("This item has no environment variables")?;
        Ok(env.to_env()?)
    }

    /// UI callers run disk writes and sync on a background executor. This owns
    /// no vault state; an explicitly started export finishes its snapshot.
    pub fn write_environment_file(path: &std::path::Path, text: &str) -> Result<()> {
        use std::io::Write as _;
        let parent = path.parent().context("Choose an export folder")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| {
            anyhow::anyhow!("Couldn't create a private export file in that folder.")
        })?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| anyhow::anyhow!("Couldn't write the environment file."))?;
        file.persist_noclobber(path)
            .map_err(|err| match err.error.kind() {
                std::io::ErrorKind::AlreadyExists => anyhow::anyhow!(
                    "That filename already exists. Choose a new filename; nothing was overwritten."
                ),
                _ => anyhow::anyhow!(
                    "Couldn't publish the export. Choose a writable location and a new filename."
                ),
            })?;
        Ok(())
    }
}

fn setup_note_title(project: &str) -> String {
    let title = format!("{project} — Setup");
    if title.chars().count() <= ottero_core::MAX_TITLE_CHARS {
        title
    } else {
        project.to_owned()
    }
}
