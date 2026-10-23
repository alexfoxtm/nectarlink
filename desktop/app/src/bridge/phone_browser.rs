// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhoneBrowser`: one phone folder at a time, as a Qt list model.
//!
//! List roles, from Qt.UserRole: name, path, isDir, size, modified. Rows are
//! opened and saved by their `path` role, which stays right when a refresh
//! moves rows. `saveFinished(bool)` fires when an explicit save settles. There
//! is no preview API. `device` is empty until `open` and again once the
//! consent ends.

use std::{path::PathBuf, pin::Pin};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QUrl, QVariant};

use super::{Edit, diff};
use crate::phone_browser::{self, PhoneBrowserRow};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, enabled)]
        #[qproperty(QString, device)]
        #[qproperty(QString, path)]
        /// "hidden", "idle", "loading", "ready", "denied", "unsupported",
        /// "notFound", "invalid", "offline", or "failed".
        #[qproperty(QString, status)]
        #[qproperty(i32, count)]
        #[qproperty(i32, loaded_count, cxx_name = "loadedCount")]
        #[qproperty(bool, can_up, cxx_name = "canUp")]
        #[qproperty(bool, saving)]
        #[qproperty(bool, truncated)]
        type PhoneBrowser = super::PhoneBrowserRust;
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(
            self: Pin<&mut PhoneBrowser>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut PhoneBrowser>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(
            self: Pin<&mut PhoneBrowser>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut PhoneBrowser>);
        #[inherit]
        fn index(self: &PhoneBrowser, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut PhoneBrowser>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &PhoneBrowser, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &PhoneBrowser) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &PhoneBrowser, parent: &QModelIndex) -> i32;

        /// Only for the user's explicit "Browse shared folders" action: it is
        /// the consent every later listing and save needs, until `close`.
        #[qinvokable]
        fn open(self: &PhoneBrowser, device: &QString);
        #[qinvokable]
        fn up(self: &PhoneBrowser);
        /// Opens the listed folder with this `path` role.
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: &PhoneBrowser, path: &QString);
        /// Filters only names already loaded; never starts a storage request.
        #[qinvokable]
        #[cxx_name = "setFilter"]
        fn set_filter(self: Pin<&mut PhoneBrowser>, query: &QString);
        #[qinvokable]
        fn refresh(self: &PhoneBrowser);
        #[qinvokable]
        fn close(self: &PhoneBrowser);
        /// Saves the listed file with this `path` role into a local folder
        /// (a `file:` URL or an absolute path).
        #[qinvokable]
        #[cxx_name = "saveFile"]
        fn save_file(self: &PhoneBrowser, path: &QString, folder_url: &QString);

        #[qsignal]
        #[cxx_name = "saveFinished"]
        fn save_finished(self: Pin<&mut PhoneBrowser>, ok: bool);
    }

    impl cxx_qt::Threading for PhoneBrowser {}
    impl cxx_qt::Initialize for PhoneBrowser {}
}

#[derive(Default)]
pub struct PhoneBrowserRust {
    enabled: bool,
    device: QString,
    path: QString,
    status: QString,
    count: i32,
    loaded_count: i32,
    filter: String,
    can_up: bool,
    saving: bool,
    truncated: bool,
    rows: Vec<PhoneBrowserRow>,
}

const ROLES: &[&str] = &["name", "path", "isDir", "size", "modified"];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &PhoneBrowserRow, role: &str) -> QVariant {
    match role {
        "name" => QVariant::from(&QString::from(&row.name)),
        "path" => QVariant::from(&QString::from(&row.path)),
        "isDir" => QVariant::from(&row.is_dir),
        "size" => QVariant::from(&(row.size as f64)),
        "modified" => QVariant::from(&(row.modified as f64)),
        _ => QVariant::default(),
    }
}

/// The local folder a folder dialog chose, only as an absolute path: any other
/// URL or a relative path would land somewhere the user didn't pick.
fn folder_from_url(url: &QString) -> Option<PathBuf> {
    let raw = String::from(url);
    if raw.trim().is_empty() {
        return None;
    }
    let local =
        QUrl::from(url).to_local_file().map(|local| String::from(&local)).filter(|text| !text.is_empty());
    let folder = PathBuf::from(local.unwrap_or(raw));
    folder.is_absolute().then_some(folder)
}

impl cxx_qt::Initialize for qobject::PhoneBrowser {
    fn initialize(mut self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        phone_browser::set_ui_listener(move || {
            let _ = qt.queue(|object| object.update_view());
        });
        self.as_mut().set_enabled(phone_browser::ENABLED);
        self.update_view();
    }
}

impl qobject::PhoneBrowser {
    fn update_view(mut self: Pin<&mut Self>) {
        let mut view = phone_browser::view();
        // A filter belongs to one consented folder, never to another phone.
        let device = QString::from(&view.device.map(|id| id.to_string()).unwrap_or_default());
        if self.device != device || String::from(&self.path) != view.path || view.device.is_none() {
            self.as_mut().rust_mut().filter.clear();
        }
        self.as_mut().set_loaded_count(view.rows.len() as i32);
        let query = self.filter.to_lowercase();
        if !query.is_empty() {
            view.rows.retain(|row| row.name.to_lowercase().contains(&query));
        }
        self.as_mut().set_enabled(view.enabled);
        self.as_mut().set_device(QString::from(&view.device.map(|id| id.to_string()).unwrap_or_default()));
        self.as_mut().set_path(QString::from(&view.path));
        self.as_mut().set_status(QString::from(view.status.as_str()));
        self.as_mut().set_can_up(view.can_up);
        self.as_mut().set_saving(view.saving);
        self.as_mut().set_truncated(view.truncated);

        let root = QModelIndex::default();
        for edit in diff(&self.rows, &view.rows, |row| row.path.clone()) {
            match edit {
                Edit::Remove(row) => {
                    let at = row as i32;
                    // SAFETY: `row` still exists; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_remove_rows(&root, at, at);
                        self.as_mut().rust_mut().rows.remove(row);
                        self.as_mut().end_remove_rows();
                    }
                }
                Edit::Insert(row) => {
                    let at = row as i32;
                    // SAFETY: `row` is <= len; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_insert_rows(&root, at, at);
                        self.as_mut().rust_mut().rows.insert(row, view.rows[row].clone());
                        self.as_mut().end_insert_rows();
                    }
                }
                Edit::Change(row) => {
                    self.as_mut().rust_mut().rows[row] = view.rows[row].clone();
                    let index = self.index(row as i32, 0, &root);
                    self.as_mut().data_changed(&index, &index, &QList::default());
                }
            }
        }
        let count = self.rows.len() as i32;
        self.as_mut().set_count(count);
        if let Some(ok) = phone_browser::take_save_notice() {
            self.as_mut().save_finished(ok);
        }
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = usize::try_from(index.row()).ok();
        let role = usize::try_from(role - USER_ROLE).ok().and_then(|slot| ROLES.get(slot));
        match (row.and_then(|index| self.rows.get(index)), role) {
            (Some(row), Some(role)) => role_value(row, role),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (index, name) in ROLES.iter().enumerate() {
            roles.insert(USER_ROLE + index as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn open(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            phone_browser::open(device);
        }
    }

    pub fn up(&self) {
        phone_browser::up();
    }

    pub fn open_folder(&self, path: &QString) {
        phone_browser::open_folder(&String::from(path));
    }

    pub fn set_filter(mut self: Pin<&mut Self>, query: &QString) {
        self.as_mut().rust_mut().filter = String::from(query).trim().to_owned();
        self.update_view();
    }

    pub fn refresh(&self) {
        phone_browser::refresh();
    }

    pub fn close(&self) {
        phone_browser::close();
    }

    pub fn save_file(&self, path: &QString, folder_url: &QString) {
        phone_browser::save_file(&String::from(path), folder_from_url(folder_url));
    }
}
