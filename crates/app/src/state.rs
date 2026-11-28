//! Everything the screens show, and how background results change it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nuvio_core::account::{self, Account, AuthError, Server, Session, SignUp};
use nuvio_core::addon::{CatalogDef, Manifest};
use nuvio_core::client::{HOME_PREVIEW_ITEMS, StreamSource};
use nuvio_core::meta::{MetaDetails, MetaPreview};
use nuvio_core::store::{AddonEntry, AddonList, CINEMETA, JsonFile, Library, ProgressBook, SearchHistory, Settings, unix_seconds};
use nuvio_core::stream::{Facet, Stream};

use crate::services::{AuthOutcome, Msg, Services};

/// After a renewal that could not be delivered (no network), the next try waits this long.
const REFRESH_RETRY: Duration = Duration::from_secs(60);

/// A cached shelf younger than this is shown without asking the addon again.
const CATALOG_FRESH: Duration = Duration::from_secs(10 * 60);
/// How many catalogs load at once.
const CATALOG_PARALLEL: usize = 6;

#[derive(Debug, Clone)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct AddonState {
    pub entry: AddonEntry,
    pub manifest: Option<Manifest>,
    pub status: Load<()>,
}

impl AddonState {
    pub fn display_name(&self) -> String {
        self.entry
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| self.manifest.as_ref().map(|m| m.name.clone()))
            .unwrap_or_else(|| host_of(&self.entry.manifest_url))
    }

    pub fn is_active(&self) -> bool {
        self.entry.enabled && self.manifest.is_some()
    }
}

/// `host` of a URL, for naming an addon whose manifest has not loaded.
pub fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|rest| rest.split(['/', '?']).next()).unwrap_or("addon").to_owned()
}

/// One shelf of the home screen: a catalog of an addon.
pub struct Section {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub def: CatalogDef,
    pub manifest_url: String,
    pub items: Vec<MetaPreview>,
    pub state: Load<()>,
    pub has_more: bool,
    pub skip: usize,
    pub loading_more: bool,
}

pub struct DetailsState {
    pub kind: String,
    pub id: String,
    pub preview: Option<MetaPreview>,
    pub meta: Load<Arc<MetaDetails>>,
    pub season: Option<i64>,
    pub request: u64,
}

pub struct StreamGroup {
    pub addon_url: String,
    pub state: Load<usize>,
    pub streams: Vec<Stream>,
}

pub struct StreamsState {
    pub kind: String,
    pub title_id: String,
    pub video_id: String,
    pub name: String,
    pub episode_label: Option<String>,
    pub episode_title: Option<String>,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub meta: Option<Arc<MetaDetails>>,
    pub request: u64,
    pub groups: Vec<StreamGroup>,
    pub selection: BTreeMap<Facet, String>,
}

impl StreamsState {
    pub fn all_streams(&self) -> Vec<&Stream> {
        self.groups.iter().flat_map(|g| g.streams.iter()).collect()
    }

    pub fn pending(&self) -> usize {
        self.groups.iter().filter(|g| matches!(g.state, Load::Loading)).count()
    }
}

pub struct SearchRow {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub items: Vec<MetaPreview>,
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub submitted: String,
    pub request: u64,
    pub rows: Vec<SearchRow>,
    pub pending: usize,
    pub typed_at: Option<Instant>,
    /// When the words in `submitted` were sent to the addons, and whether they are in the history already.
    pub ran_at: Option<Instant>,
    pub remembered: bool,
}

/// The signed-in person, and what the sign-in form is doing.
pub struct AccountState {
    file: JsonFile<Account>,
    pub session: Option<Session>,
    /// The form's request is out.
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    request: u64,
    refreshing: bool,
    retry_at: Option<Instant>,
}

impl AccountState {
    fn load() -> Self {
        let file = JsonFile::<Account>::private("account.json");
        Self { session: file.load().session, file, busy: false, error: None, notice: None, request: 0, refreshing: false, retry_at: None }
    }

    fn set_session(&mut self, session: Option<Session>) {
        self.session = session;
        self.file.save(&Account { session: self.session.clone() });
    }

    /// When the access token needs renewing: now if it is due and no renewal is out or waiting to be retried.
    pub fn refresh_due(&self) -> RefreshDue {
        let Some(session) = &self.session else { return RefreshDue::Never };
        if self.refreshing {
            return RefreshDue::Never;
        }
        let now = unix_seconds();
        if let Some(at) = self.retry_at.filter(|at| *at > Instant::now()) {
            return RefreshDue::In(at - Instant::now());
        }
        if session.needs_refresh(now) { RefreshDue::Now } else { RefreshDue::In(Duration::from_secs(session.refresh_in(now) + 1)) }
    }
}

pub enum RefreshDue {
    Never,
    Now,
    In(Duration),
}

pub struct State {
    pub addon_file: JsonFile<AddonList>,
    pub addon_list: AddonList,
    pub addons: Vec<AddonState>,
    pub sections: Vec<Section>,
    pub details: Option<DetailsState>,
    pub streams: Option<StreamsState>,
    pub search: SearchState,
    pub library_file: JsonFile<Library>,
    pub library: Library,
    pub progress_file: JsonFile<ProgressBook>,
    pub progress: ProgressBook,
    pub history_file: JsonFile<SearchHistory>,
    pub history: SearchHistory,
    pub settings_file: JsonFile<Settings>,
    pub settings: Settings,
    pub account: AccountState,
    pub next_request: u64,
    pub kotlin_addons: Vec<String>,
    catalog_inflight: usize,
    catalog_queue: Vec<String>,
}

impl State {
    pub fn load() -> Self {
        let addon_file = JsonFile::<AddonList>::new("addons.json");
        let mut addon_list = addon_file.load();
        if !addon_list.defaults_installed {
            addon_list.add(CINEMETA.to_owned());
            addon_list.defaults_installed = true;
            addon_file.save(&addon_list);
        }
        let library_file = JsonFile::<Library>::new("library.json");
        let progress_file = JsonFile::<ProgressBook>::new("progress.json");
        let settings_file = JsonFile::<Settings>::new("settings.json");
        let history_file = JsonFile::<SearchHistory>::new("search_history.json");
        Self {
            library: library_file.load(),
            progress: progress_file.load(),
            settings: settings_file.load(),
            history: history_file.load(),
            history_file,
            library_file,
            progress_file,
            settings_file,
            account: AccountState::load(),
            addon_file,
            addons: Vec::new(),
            addon_list,
            sections: Vec::new(),
            details: None,
            streams: None,
            search: SearchState::default(),
            next_request: 1,
            kotlin_addons: Vec::new(),
            catalog_inflight: 0,
            catalog_queue: Vec::new(),
        }
    }

    pub fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    /// Signs in with the address and password, or makes the account. The answer arrives as `Msg::Auth`.
    pub fn sign_in(&mut self, svc: &Services, email: String, password: String, create: bool) {
        if self.account.busy {
            return;
        }
        self.account.busy = true;
        self.account.error = None;
        self.account.notice = None;
        let request = self.request_id();
        self.account.request = request;
        let http = svc.client.http().clone();
        svc.spawn(async move {
            let result = async {
                let server = Server::resolve(&http).await?;
                if create {
                    Ok(match account::sign_up(&http, &server, &email, &password).await? {
                        SignUp::SignedIn(session) => AuthOutcome::SignedIn(session),
                        SignUp::ConfirmEmail => AuthOutcome::ConfirmEmail,
                    })
                } else {
                    account::sign_in(&http, &server, &email, &password).await.map(AuthOutcome::SignedIn)
                }
            }
            .await
            .map_err(|error: AuthError| error.to_string());
            Msg::Auth { request, result }
        });
    }

    /// Forgets the session here at once and tells the server in the background.
    pub fn sign_out(&mut self, svc: &Services) {
        self.account.error = None;
        self.account.notice = None;
        if let Some(session) = self.account.session.clone() {
            self.account.set_session(None);
            let http = svc.client.http().clone();
            svc.runtime.spawn(async move { account::sign_out(&http, &session).await });
        }
    }

    /// Renews the access token before it runs out. The answer arrives as `Msg::SessionRefreshed`.
    pub fn refresh_session(&mut self, svc: &Services) {
        let Some(session) = self.account.session.clone() else { return };
        self.account.refreshing = true;
        let http = svc.client.http().clone();
        svc.spawn(async move {
            let user_id = session.user_id.clone();
            Msg::SessionRefreshed { user_id, result: account::refresh(&http, &session).await }
        });
    }

    pub fn save_settings(&self) {
        self.settings_file.save(&self.settings);
    }

    pub fn save_library(&self) {
        self.library_file.save(&self.library);
    }

    pub fn save_progress(&self) {
        self.progress_file.save(&self.progress);
    }

    pub fn save_history(&self) {
        self.history_file.save(&self.history);
    }

    /// Remembers what was searched for, unless it already is the latest.
    pub fn remember_search(&mut self, query: &str) {
        self.search.remembered = true;
        let query = query.trim();
        if self.history.entries.first().is_some_and(|latest| latest == query) {
            return;
        }
        self.history.record(query);
        self.save_history();
    }

    pub fn save_addons(&self) {
        self.addon_file.save(&self.addon_list);
    }

    // -- addons ----------------------------------------------------------------------------------------------------

    /// Brings `addons` in line with the saved list, shows whatever manifests are cached, and asks every addon for a
    /// fresh one.
    pub fn start_addons(&mut self, svc: &Services) {
        self.addons = self
            .addon_list
            .addons
            .iter()
            .map(|entry| {
                let manifest = svc.client.cached_manifest(&entry.manifest_url);
                AddonState { entry: entry.clone(), status: if manifest.is_some() { Load::Ready(()) } else { Load::Loading }, manifest }
            })
            .collect();
        // A test run can add an addon that is never saved: its URL holds a personal token.
        if let Ok(url) = std::env::var("NUVIO_TEST_ADDON_URL") {
            self.addons.push(AddonState { entry: AddonEntry { manifest_url: url, enabled: true, name: None }, manifest: None, status: Load::Loading });
        }
        self.rebuild_sections(svc);
        for addon in &self.addons {
            self.spawn_manifest(svc, &addon.entry.manifest_url);
        }
    }

    fn spawn_manifest(&self, svc: &Services, url: &str) {
        let (client, url) = (svc.client.clone(), url.to_owned());
        svc.spawn(async move {
            let result = client.manifest(&url).await.map_err(|e| e.to_string());
            Msg::Manifest { url, result }
        });
    }

    pub fn add_addon(&mut self, svc: &Services, manifest_url: String) -> bool {
        if !self.addon_list.add(manifest_url.clone()) {
            return false;
        }
        self.save_addons();
        self.addons.push(AddonState {
            entry: AddonEntry { manifest_url: manifest_url.clone(), enabled: true, name: None },
            manifest: None,
            status: Load::Loading,
        });
        self.spawn_manifest(svc, &manifest_url);
        true
    }

    pub fn remove_addon(&mut self, svc: &Services, manifest_url: &str) {
        self.addon_list.addons.retain(|a| a.manifest_url != manifest_url);
        self.addons.retain(|a| a.entry.manifest_url != manifest_url);
        self.save_addons();
        self.rebuild_sections(svc);
    }

    pub fn set_addon_enabled(&mut self, svc: &Services, manifest_url: &str, enabled: bool) {
        for entry in self.addon_list.addons.iter_mut().filter(|a| a.manifest_url == manifest_url) {
            entry.enabled = enabled;
        }
        for addon in self.addons.iter_mut().filter(|a| a.entry.manifest_url == manifest_url) {
            addon.entry.enabled = enabled;
        }
        self.save_addons();
        self.rebuild_sections(svc);
    }

    pub fn move_addon(&mut self, svc: &Services, manifest_url: &str, delta: isize) {
        let Some(index) = self.addon_list.addons.iter().position(|a| a.manifest_url == manifest_url) else { return };
        let target = index.saturating_add_signed(delta).min(self.addon_list.addons.len() - 1);
        self.addon_list.addons.swap(index, target);
        self.addons.swap(index, target);
        self.save_addons();
        self.rebuild_sections(svc);
    }

    // -- home ------------------------------------------------------------------------------------------------------

    /// Recreates the shelves from the addons' catalogs, keeping the items of shelves that are still there, and loads
    /// the ones that are empty or stale.
    pub fn rebuild_sections(&mut self, svc: &Services) {
        let mut old: BTreeMap<String, Section> = std::mem::take(&mut self.sections).into_iter().map(|s| (s.key.clone(), s)).collect();
        let show_type = self.settings.show_catalog_type;
        let mut fresh = Vec::new();
        for addon in self.addons.iter().filter(|a| a.entry.enabled) {
            let Some(manifest) = &addon.manifest else { continue };
            for def in manifest.catalogs.iter().filter(|c| c.is_browsable()) {
                let key = format!("{}|{}|{}", addon.entry.manifest_url, def.kind, def.id);
                let title = if show_type { format!("{} - {}", def.name, type_label(&def.kind)) } else { def.name.clone() };
                let mut section = old.remove(&key).unwrap_or_else(|| Section {
                    key: key.clone(),
                    title: String::new(),
                    subtitle: String::new(),
                    def: def.clone(),
                    manifest_url: addon.entry.manifest_url.clone(),
                    items: Vec::new(),
                    state: Load::Loading,
                    has_more: false,
                    skip: 0,
                    loading_more: false,
                });
                section.title = title;
                section.subtitle = addon.display_name();
                section.def = def.clone();
                fresh.push(section);
            }
        }
        self.sections = fresh;
        self.catalog_queue.clear();
        self.catalog_inflight = 0;
        let keys: Vec<String> = self.sections.iter().map(|s| s.key.clone()).collect();
        for key in keys {
            self.queue_catalog(svc, &key, false);
        }
        self.pump_catalogs(svc);
    }

    pub fn refresh_home(&mut self, svc: &Services) {
        self.catalog_queue.clear();
        let keys: Vec<String> = self.sections.iter().map(|s| s.key.clone()).collect();
        for key in keys {
            self.queue_catalog(svc, &key, true);
        }
        self.pump_catalogs(svc);
    }

    fn queue_catalog(&mut self, svc: &Services, key: &str, force: bool) {
        let Some(section) = self.sections.iter_mut().find(|s| s.key == key) else { return };
        let cached = svc.client.cached_catalog(&section.manifest_url, &section.def, Some(HOME_PREVIEW_ITEMS));
        let mut needs_network = true;
        if let Some((page, age)) = cached {
            if section.items.is_empty() {
                section.has_more = section.def.supports_skip() && page.raw_count > 0;
                section.items = page.items;
                section.state = Load::Ready(());
            }
            needs_network = force || age > CATALOG_FRESH;
        }
        if needs_network {
            self.catalog_queue.push(key.to_owned());
        } else if section.items.is_empty() {
            section.state = Load::Ready(());
        }
    }

    /// Starts queued catalog requests, a few at a time, in the order the shelves are shown.
    fn pump_catalogs(&mut self, svc: &Services) {
        while self.catalog_inflight < CATALOG_PARALLEL && !self.catalog_queue.is_empty() {
            let key = self.catalog_queue.remove(0);
            let Some(section) = self.sections.iter().find(|s| s.key == key) else { continue };
            let (client, manifest_url, def) = (svc.client.clone(), section.manifest_url.clone(), section.def.clone());
            self.catalog_inflight += 1;
            svc.spawn(async move {
                let result = client.catalog(&manifest_url, &def, 0, Some(HOME_PREVIEW_ITEMS)).await.map_err(|e| e.to_string());
                Msg::Catalog { key, result }
            });
        }
    }

    pub fn load_more(&mut self, svc: &Services, key: &str) {
        let Some(section) = self.sections.iter_mut().find(|s| s.key == key) else { return };
        if section.loading_more || !section.has_more {
            return;
        }
        section.loading_more = true;
        let skip = section.items.len().max(section.skip);
        let (client, manifest_url, def, key) = (svc.client.clone(), section.manifest_url.clone(), section.def.clone(), key.to_owned());
        svc.spawn(async move {
            let result = client.catalog(&manifest_url, &def, skip, None).await.map_err(|e| e.to_string());
            Msg::CatalogMore { key, skip, result }
        });
    }

    // -- details ---------------------------------------------------------------------------------------------------

    pub fn open_details(&mut self, svc: &Services, kind: &str, id: &str, preview: Option<MetaPreview>) {
        let request = self.request_id();
        self.details = Some(DetailsState {
            kind: kind.to_owned(),
            id: id.to_owned(),
            preview,
            meta: Load::Loading,
            season: None,
            request,
        });
        // Addons that serve meta for this id, in the order the user arranged them.
        let sources: Vec<String> = self
            .addons
            .iter()
            .filter(|a| a.is_active() && a.manifest.as_ref().is_some_and(|m| m.supports("meta", kind, id)))
            .map(|a| a.entry.manifest_url.clone())
            .collect();
        if sources.is_empty() {
            if let Some(details) = &mut self.details {
                details.meta = match &details.preview {
                    Some(preview) => Load::Ready(Arc::new(minimal_meta(preview))),
                    None => Load::Failed("No installed addon provides details for this title.".into()),
                };
            }
            return;
        }
        // Show the cached copy at once, then replace it with the fresh one.
        if let Some((meta, _)) = sources.iter().find_map(|url| svc.client.cached_meta(url, kind, id)) {
            if let Some(details) = &mut self.details {
                details.season = default_season(&meta, &self.progress);
                details.meta = Load::Ready(Arc::new(meta));
            }
        }
        let (client, kind, id) = (svc.client.clone(), kind.to_owned(), id.to_owned());
        svc.spawn(async move {
            let mut last = String::from("no addon answered");
            for url in sources {
                match client.meta(&url, &kind, &id).await {
                    Ok(meta) => return Msg::Meta { request, result: Ok(meta) },
                    Err(error) => last = error.to_string(),
                }
            }
            Msg::Meta { request, result: Err(last) }
        });
    }

    // -- streams ---------------------------------------------------------------------------------------------------

    /// Asks every addon that serves streams for `video_id` (a film id, or `tt1:2:3` for an episode).
    pub fn open_streams(&mut self, svc: &Services, mut state: StreamsState) {
        let request = self.request_id();
        state.request = request;
        let kind = state.kind.clone();
        let video_id = state.video_id.clone();
        state.groups = self
            .addons
            .iter()
            .filter(|a| a.is_active() && a.manifest.as_ref().is_some_and(|m| m.supports("stream", &kind, &video_id)))
            .map(|a| StreamGroup { addon_url: a.entry.manifest_url.clone(), state: Load::Loading, streams: Vec::new() })
            .collect();
        for addon in self.addons.iter().filter(|a| a.is_active() && a.manifest.as_ref().is_some_and(|m| m.supports("stream", &kind, &video_id))) {
            let manifest = addon.manifest.as_ref();
            let source = StreamSource {
                manifest_url: addon.entry.manifest_url.clone(),
                name: addon.display_name(),
                id: manifest.map(|m| m.id.clone()).unwrap_or_default(),
                logo: manifest.and_then(|m| m.logo.clone()),
            };
            let (client, kind, video_id) = (svc.client.clone(), kind.clone(), video_id.clone());
            svc.spawn(async move {
                let addon_url = source.manifest_url.clone();
                let result = client.streams(&source, &kind, &video_id).await.map_err(|e| e.to_string());
                Msg::Streams { request, addon_url, result }
            });
        }
        self.streams = Some(state);
    }

    // -- search ----------------------------------------------------------------------------------------------------

    pub fn run_search(&mut self, svc: &Services) {
        let query = self.search.query.trim().to_owned();
        self.search.typed_at = None;
        self.search.submitted = query.clone();
        self.search.rows.clear();
        self.search.pending = 0;
        self.search.ran_at = None;
        self.search.remembered = false;
        if query.chars().count() < 2 {
            return;
        }
        self.search.ran_at = Some(Instant::now());
        let request = self.request_id();
        self.search.request = request;
        for addon in self.addons.iter().filter(|a| a.is_active()) {
            let Some(manifest) = &addon.manifest else { continue };
            for def in manifest.catalogs.iter().filter(|c| c.supports_search()) {
                self.search.pending += 1;
                let (client, url, def, query) = (svc.client.clone(), addon.entry.manifest_url.clone(), def.clone(), query.clone());
                let title = format!("{} - {}", def.name, type_label(&def.kind));
                let subtitle = addon.display_name();
                let key = format!("{url}|{}|{}", def.kind, def.id);
                svc.spawn(async move {
                    let result = client.search(&url, &def, &query).await.map_err(|e| e.to_string());
                    Msg::Search { request, key, title, subtitle, result }
                });
            }
        }
    }

    // -- results ---------------------------------------------------------------------------------------------------

    pub fn apply(&mut self, message: Msg, svc: &Services) {
        match message {
            Msg::Manifest { url, result } => {
                let mut changed = false;
                if let Some(addon) = self.addons.iter_mut().find(|a| a.entry.manifest_url == url) {
                    match result {
                        Ok(manifest) => {
                            changed = addon.manifest.as_ref() != Some(&manifest);
                            addon.manifest = Some(manifest);
                            addon.status = Load::Ready(());
                        }
                        Err(error) => {
                            if addon.manifest.is_none() {
                                addon.status = Load::Failed(error);
                            }
                        }
                    }
                }
                if changed {
                    self.rebuild_sections(svc);
                }
            }
            Msg::Catalog { key, result } => {
                self.catalog_inflight = self.catalog_inflight.saturating_sub(1);
                if let Some(section) = self.sections.iter_mut().find(|s| s.key == key) {
                    match result {
                        Ok(page) => {
                            section.has_more = section.def.supports_skip() && page.raw_count > 0;
                            section.skip = page.raw_count;
                            section.items = page.items;
                            section.state = Load::Ready(());
                        }
                        Err(error) if section.items.is_empty() => section.state = Load::Failed(error),
                        Err(_) => {}
                    }
                }
                self.pump_catalogs(svc);
            }
            Msg::CatalogMore { key, skip, result } => {
                if let Some(section) = self.sections.iter_mut().find(|s| s.key == key) {
                    section.loading_more = false;
                    match result {
                        Ok(page) => {
                            section.has_more = page.raw_count > 0;
                            section.skip = skip + page.raw_count;
                            let known: std::collections::HashSet<String> = section.items.iter().map(MetaPreview::key).collect();
                            section.items.extend(page.items.into_iter().filter(|i| !known.contains(&i.key())));
                        }
                        Err(_) => section.has_more = false,
                    }
                }
            }
            Msg::Meta { request, result } => {
                if let Some(details) = self.details.as_mut().filter(|d| d.request == request) {
                    match result {
                        Ok(meta) => {
                            if details.season.is_none() {
                                details.season = default_season(&meta, &self.progress);
                            }
                            details.meta = Load::Ready(Arc::new(meta));
                        }
                        Err(error) => {
                            if details.meta.ready().is_none() {
                                details.meta = match &details.preview {
                                    Some(preview) => Load::Ready(Arc::new(minimal_meta(preview))),
                                    None => Load::Failed(error),
                                };
                            }
                        }
                    }
                }
            }
            Msg::Streams { request, addon_url, result } => {
                if let Some(streams) = self.streams.as_mut().filter(|s| s.request == request) {
                    if let Some(group) = streams.groups.iter_mut().find(|g| g.addon_url == addon_url) {
                        match result {
                            Ok(list) => {
                                group.state = Load::Ready(list.len());
                                group.streams = list;
                            }
                            Err(error) => group.state = Load::Failed(error),
                        }
                    }
                }
            }
            Msg::Search { request, key, title, subtitle, result } => {
                if self.search.request == request {
                    self.search.pending = self.search.pending.saturating_sub(1);
                    if let Ok(page) = result {
                        if !page.items.is_empty() {
                            self.search.rows.push(SearchRow { key, title, subtitle, items: page.items });
                        }
                    }
                }
            }
            Msg::Subtitles { .. } | Msg::SubtitleFile { .. } | Msg::Segments { .. } => {}
            Msg::KotlinAddons(urls) => self.kotlin_addons = urls,
            Msg::Auth { request, result } if request == self.account.request => {
                self.account.busy = false;
                match result {
                    Ok(AuthOutcome::SignedIn(session)) => {
                        self.account.set_session(Some(session));
                        self.account.retry_at = None;
                        self.account.error = None;
                    }
                    Ok(AuthOutcome::ConfirmEmail) => {
                        self.account.notice = Some("Account created. Open the link in the email that was sent to you, then sign in.".into());
                    }
                    Err(error) => self.account.error = Some(error),
                }
            }
            Msg::Auth { .. } => {}
            Msg::SessionRefreshed { user_id, result } => {
                self.account.refreshing = false;
                // The person may have signed out, or into another account, while this was on its way.
                if self.account.session.as_ref().is_none_or(|s| s.user_id != user_id) {
                    return;
                }
                match result {
                    Ok(session) => {
                        // A server that hands out tokens shorter than the renewal margin must not be asked in a loop.
                        self.account.retry_at = session.needs_refresh(unix_seconds()).then(|| Instant::now() + REFRESH_RETRY);
                        self.account.set_session(Some(session));
                    }
                    Err(error) if error.ends_session() => {
                        tracing::info!("the account session was refused, signing out");
                        self.account.set_session(None);
                        self.account.error = Some("Your session has ended. Sign in again.".into());
                    }
                    Err(error) => {
                        tracing::debug!(%error, "could not renew the session, will retry");
                        self.account.retry_at = Some(Instant::now() + REFRESH_RETRY);
                    }
                }
            }
            // Torrent streams belong to the player session, which the app owns.
            Msg::Torrent { .. } => {}
        }
    }

    /// The addons that can offer subtitles for a video.
    pub fn subtitle_sources(&self, kind: &str, video_id: &str) -> Vec<StreamSource> {
        self.addons
            .iter()
            .filter(|a| a.is_active())
            .filter_map(|a| {
                let manifest = a.manifest.as_ref().filter(|m| m.supports("subtitles", kind, video_id))?;
                Some(StreamSource { manifest_url: a.entry.manifest_url.clone(), name: a.display_name(), id: manifest.id.clone(), logo: None })
            })
            .collect()
    }

}

pub fn type_label(kind: &str) -> String {
    match kind {
        "movie" => "Movies".into(),
        "series" => "Series".into(),
        "anime" => "Anime".into(),
        "tv" => "TV".into(),
        "channel" => "Channels".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    }
}

fn minimal_meta(preview: &MetaPreview) -> MetaDetails {
    MetaDetails {
        id: preview.id.clone(),
        kind: preview.kind.clone(),
        name: preview.name.clone(),
        poster: preview.poster.clone(),
        background: preview.background.clone(),
        logo: preview.logo.clone(),
        description: preview.description.clone(),
        release_info: preview.release_info.clone(),
        imdb_rating: preview.imdb_rating.clone(),
        genres: preview.genres.clone(),
        ..MetaDetails::default()
    }
}

/// The season to open on: the one of the last episode watched, else the first real season.
fn default_season(meta: &MetaDetails, progress: &ProgressBook) -> Option<i64> {
    let latest = progress
        .entries
        .values()
        .filter(|p| p.id == meta.id && p.kind == meta.kind)
        .max_by_key(|p| p.updated_at)
        .and_then(|p| p.season);
    let seasons = meta.seasons();
    latest.filter(|s| seasons.contains(s)).or_else(|| seasons.iter().copied().find(|s| *s > 0)).or_else(|| seasons.first().copied())
}
