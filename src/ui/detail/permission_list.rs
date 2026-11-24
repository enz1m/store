//! The read-only Permissions dialog's body: what an app may reach, in words.
//!
//! Each declared permission becomes a row with an icon, a plain name and a
//! line saying what it allows, grouped by kind. Broad access (the network,
//! the home folder and other files outside the app's own, input and other
//! devices, X11, the whole of D-Bus, and services that run commands outside
//! the sandbox) is marked with a warning-toned icon. The detail page's summary
//! and badges are built from the same items.

use super::*;
use iced::widget::column;

/// One permission, described.
pub(crate) struct PermissionItem {
    pub icon: &'static str,
    pub title: String,
    pub detail: String,
    /// Access wide enough to note: it reaches beyond the app's own files.
    pub broad: bool,
}

fn item(
    icon: &'static str,
    title: &'static str,
    detail: &'static str,
    broad: bool,
) -> PermissionItem {
    PermissionItem {
        icon,
        title: crate::i18n::label(title).to_owned(),
        detail: crate::i18n::label(detail).to_owned(),
        broad,
    }
}

fn raw(icon: &'static str, title: String, detail: String, broad: bool) -> PermissionItem {
    PermissionItem {
        icon,
        title,
        detail,
        broad,
    }
}

pub(crate) fn describe_shared(value: &str) -> PermissionItem {
    match value {
        "network" => item(
            "globe",
            "Network",
            "Use the internet and your local network.",
            true,
        ),
        "ipc" => item(
            "share-2",
            "Shared memory with other apps",
            "Needed by X11 apps to draw efficiently.",
            false,
        ),
        other => raw("share-2", other.to_owned(), String::new(), false),
    }
}

pub(crate) fn describe_socket(value: &str) -> PermissionItem {
    match value {
        "wayland" => item("app-window", "Wayland display", "Show its windows.", false),
        "x11" => item(
            "app-window",
            "X11 display",
            "Show its windows the older way, which lets it see other X11 windows and keystrokes.",
            true,
        ),
        "fallback-x11" => item(
            "app-window",
            "X11 display when Wayland is missing",
            "Only used where there is no Wayland.",
            false,
        ),
        "pulseaudio" => item("volume-2", "Sound", "Play and record audio.", false),
        "pcsc" => item(
            "credit-card",
            "Smart cards",
            "Use smart card readers.",
            false,
        ),
        "cups" => item("printer", "Printing", "Print documents.", false),
        "ssh-auth" => item(
            "key",
            "SSH agent",
            "Use your SSH keys to sign in to servers.",
            true,
        ),
        "gpg-agent" => item(
            "key",
            "GPG agent",
            "Use your GPG keys to sign and decrypt.",
            true,
        ),
        "session-bus" => item(
            "message-square",
            "All desktop services",
            "Talk to every service on your desktop session.",
            true,
        ),
        "system-bus" => item(
            "message-square",
            "All system services",
            "Talk to every system service.",
            true,
        ),
        other => raw("cable", other.to_owned(), String::new(), false),
    }
}

pub(crate) fn describe_device(value: &str) -> PermissionItem {
    match value {
        "all" => item(
            "cpu",
            "All devices",
            "Cameras, microphones, game controllers, USB devices and more.",
            true,
        ),
        "dri" => item(
            "monitor",
            "Graphics acceleration",
            "Use the graphics card.",
            false,
        ),
        "kvm" => item("cpu", "Virtualization", "Run virtual machines.", false),
        "shm" => item(
            "memory-stick",
            "Shared memory",
            "Share memory with other apps.",
            false,
        ),
        "input" => item(
            "keyboard",
            "Input devices",
            "Read keyboards, mice and game controllers directly, including what you type in other apps.",
            true,
        ),
        "usb" => item("usb", "USB devices", "Use USB devices directly.", false),
        other => raw("cpu", other.to_owned(), String::new(), false),
    }
}

pub(crate) fn describe_feature(value: &str) -> PermissionItem {
    match value {
        "devel" => item("bug", "Development tools", "Debug other programs.", true),
        "multiarch" => item(
            "cpu",
            "32-bit programs",
            "Run programs built for other architectures.",
            false,
        ),
        "bluetooth" => item("bluetooth", "Bluetooth", "Use Bluetooth devices.", false),
        "canbus" => item("cable", "CAN bus", "Use CAN bus devices.", false),
        "per-app-dev-shm" => item(
            "memory-stick",
            "Shared memory per app",
            "Keep shared memory for this app's own instances.",
            false,
        ),
        other => raw("layers", other.to_owned(), String::new(), false),
    }
}

pub(crate) fn describe_filesystem(value: &str) -> PermissionItem {
    let (path, mode) = match value.rsplit_once(':') {
        Some((path, mode @ ("ro" | "rw" | "create"))) => (path, mode),
        _ => (value, "rw"),
    };
    let access = crate::i18n::label(match mode {
        "ro" => "Read only.",
        "create" => "Read and write, and create it if missing.",
        _ => "Read and write.",
    })
    .to_owned();
    let named = |icon: &'static str, title: &'static str, broad: bool| PermissionItem {
        icon,
        title: crate::i18n::label(title).to_owned(),
        detail: access.clone(),
        broad,
    };
    match path {
        "host" => named("hard-drive", "All files on this computer", true),
        "host-os" => named("hard-drive", "System programs and libraries", true),
        "host-etc" => named("hard-drive", "System settings", true),
        "home" | "~" => named("house", "Your home folder", true),
        "xdg-desktop" => named("folder", "Your Desktop folder", false),
        "xdg-documents" => named("folder", "Your Documents folder", false),
        "xdg-download" => named("folder-down", "Your Downloads folder", false),
        "xdg-music" => named("music", "Your Music folder", false),
        "xdg-pictures" => named("image", "Your Pictures folder", false),
        "xdg-videos" => named("video", "Your Videos folder", false),
        "xdg-run/pipewire-0" => PermissionItem {
            icon: "video",
            title: crate::i18n::label("Audio and video streams").to_owned(),
            detail: crate::i18n::label("Use PipeWire, for screen sharing and cameras.").to_owned(),
            broad: false,
        },
        flatpak if subpath_of(flatpak, "xdg-data/flatpak") => {
            named("layers", "Flatpak apps and their permissions", true)
        }
        autostart if subpath_of(autostart, "xdg-config/autostart") => {
            named("power", "Programs started when you sign in", true)
        }
        other => PermissionItem {
            icon: "folder",
            title: other.to_owned(),
            detail: access,
            broad: escapes_app_files(other),
        },
    }
}

fn subpath_of(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

// Desktop configuration and data can hold startup programs, launchers and other apps' overrides.
fn escapes_app_files(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('~')
        || ["home", "xdg-config", "xdg-data"]
            .iter()
            .any(|root| subpath_of(path, root))
}

// Talking to these session services starts arbitrary programs outside the sandbox.
const HOST_COMMAND_SERVICES: &[&str] = &["org.freedesktop.Flatpak", "org.freedesktop.systemd1"];

fn runs_host_commands(name: &str) -> bool {
    HOST_COMMAND_SERVICES
        .iter()
        .any(|service| name == *service || name.starts_with(&format!("{service}.")))
}

// D-Bus names an app may reach: a row per bus, with services that run
// programs outside the sandbox drawn out into their own row.
fn describe_bus(entries: &[String], system: bool) -> Vec<PermissionItem> {
    let mut names = Vec::new();
    let mut host_commands = Vec::new();
    let mut wildcard = false;
    for entry in entries {
        let (name, policy) = match entry.split_once('=') {
            Some((name, policy)) => (name.trim(), policy.trim()),
            None => (entry.trim(), ""),
        };
        if policy == "none" {
            continue;
        }
        wildcard |= name.contains('*');
        let line = match policy {
            "own" => format!("{name} ({})", crate::i18n::label("provides it")),
            _ => name.to_owned(),
        };
        if !system && policy != "see" && runs_host_commands(name) {
            host_commands.push(line);
        } else {
            names.push(line);
        }
    }
    let mut items = Vec::new();
    if !host_commands.is_empty() {
        let mut detail =
            crate::i18n::label("Start any program on this computer without the sandbox's limits.")
                .to_owned();
        for name in host_commands {
            detail.push('\n');
            detail.push_str(&name);
        }
        items.push(PermissionItem {
            icon: "terminal",
            title: crate::i18n::label("Programs outside the sandbox").to_owned(),
            detail,
            broad: true,
        });
    }
    if !names.is_empty() {
        let title = if system {
            crate::i18n::label("System services")
        } else {
            crate::i18n::label("Desktop services")
        };
        items.push(PermissionItem {
            icon: "message-square",
            title: title.to_owned(),
            detail: names.join("\n"),
            broad: system || wildcard,
        });
    }
    items
}

/// The permission groups that have anything in them, in reading order.
pub(crate) fn permission_groups(perms: &Permissions) -> Vec<(&'static str, Vec<PermissionItem>)> {
    let mut groups: Vec<(&'static str, Vec<PermissionItem>)> = Vec::new();
    let mut push = |title: &'static str, mut items: Vec<PermissionItem>| {
        if !items.is_empty() {
            // Broad access leads its group, so it is read before scrolling.
            items.sort_by_key(|item| !item.broad);
            groups.push((crate::i18n::label(title), items));
        }
    };
    // `ipc` is X11's shared memory: it belongs with the display, not the
    // network.
    let (network, shared_rest): (Vec<&String>, Vec<&String>) =
        perms.shared.iter().partition(|v| v.as_str() == "network");
    push(
        "Network",
        network.into_iter().map(|v| describe_shared(v)).collect(),
    );
    push(
        "Display and sound",
        perms
            .sockets
            .iter()
            .map(|v| describe_socket(v))
            .chain(shared_rest.into_iter().map(|v| describe_shared(v)))
            .collect(),
    );
    push(
        "Devices",
        perms.devices.iter().map(|v| describe_device(v)).collect(),
    );
    push(
        "Files",
        perms
            .filesystems
            .iter()
            // A leading `!` revokes access: nothing to declare.
            .filter(|v| !v.starts_with('!'))
            .map(|v| describe_filesystem(v))
            .collect(),
    );
    push(
        "Features",
        perms.features.iter().map(|v| describe_feature(v)).collect(),
    );
    let mut services = describe_bus(&perms.session_bus, false);
    services.extend(describe_bus(&perms.system_bus, true));
    push("Services", services);
    groups
}

/// The detail page's badges: each broad permission by name, then each group
/// that also holds narrower access, with whether the badge marks broad access.
pub(crate) fn permission_badges(
    groups: &[(&'static str, Vec<PermissionItem>)],
) -> Vec<(String, bool)> {
    let broad = groups
        .iter()
        .flat_map(|(_, items)| items.iter().filter(|item| item.broad))
        .map(|item| (item.title.clone(), true));
    let narrow = groups
        .iter()
        .filter(|(_, items)| items.iter().any(|item| !item.broad))
        .map(|(title, _)| ((*title).to_owned(), false));
    broad.chain(narrow).collect()
}

/// One described permission as a list row.
fn permission_row<'a>(entry: PermissionItem) -> Element<'a, Message> {
    let broad = entry.broad;
    let icon = enzui_widgets::icon::enz_icon(entry.icon)
        .size(18.0)
        .tint_resolved(move |t| {
            let p = crate::theme::palette(t);
            if broad { p.warning } else { p.text_muted }
        })
        .into_element();
    let mut text = column![
        enz_text(entry.title)
            .size(TextSize::Body)
            .color_resolved(|t| crate::theme::palette(t).text),
    ]
    .spacing(2.0)
    .width(Length::Fill);
    if !entry.detail.is_empty() {
        text = text.push(
            enz_text(entry.detail)
                .size(TextSize::Footnote)
                .tone(TextTone::Muted),
        );
    }
    row![
        container(icon).padding(iced::Padding::new(0.0).top(1.0)),
        text
    ]
    .spacing(SpaceToken::Md.px())
    .align_y(Alignment::Start)
    .padding([SpaceToken::Sm.px(), SpaceToken::Md.px()])
    .into()
}

/// The dialog's body: a summary, then each group as a framed list.
pub(crate) fn permission_list<'a>(perms: Option<&Permissions>) -> Element<'a, Message> {
    let Some(perms) = perms else {
        return chrome::muted_line(crate::i18n::label("Permission metadata is unavailable."));
    };
    let mut body = column![
        enz_text(permission_glance(Some(perms)))
            .size(TextSize::Footnote)
            .tone(TextTone::Muted)
    ]
    .spacing(SpaceToken::Md.px())
    .width(Length::Fill);
    for (title, items) in permission_groups(perms) {
        let mut list = column![].width(Length::Fill);
        for (index, entry) in items.into_iter().enumerate() {
            if index > 0 {
                list = list.push(chrome::hairline());
            }
            list = list.push(permission_row(entry));
        }
        body = body.push(
            column![
                chrome::section_label(title),
                enz_surface(list)
                    .tone(SurfaceTone::SurfaceAlt)
                    .border_style(BorderStyle::Thin)
                    .border_width(1.0)
                    .radius(10.0)
                    .fill_width()
                    .into_element(),
            ]
            .spacing(SpaceToken::Xs.px()),
        );
    }
    body.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(build: impl FnOnce(&mut Permissions)) -> Permissions {
        let mut perms = Permissions::default();
        build(&mut perms);
        perms
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn broad_flags(perms: &Permissions) -> Vec<bool> {
        permission_groups(perms)
            .into_iter()
            .flat_map(|(_, items)| items)
            .map(|item| item.broad)
            .collect()
    }

    #[test]
    fn access_that_can_leave_the_sandbox_is_broad() {
        for filesystem in [
            "~/.ssh",
            "~/.bashrc:ro",
            "home/Projects",
            "xdg-config",
            "xdg-config/autostart:create",
            "xdg-config/systemd",
            "xdg-data/flatpak/overrides",
            "xdg-data/applications",
            "/etc",
        ] {
            let p = perms(|p| p.filesystems = strings(&[filesystem]));
            assert_eq!(broad_flags(&p), [true], "{filesystem}");
        }
        for device in ["all", "input"] {
            let p = perms(|p| p.devices = strings(&[device]));
            assert_eq!(broad_flags(&p), [true], "{device}");
        }
        for policy in [
            "org.freedesktop.Flatpak = talk",
            "org.freedesktop.systemd1 = talk",
            "org.freedesktop.* = talk",
        ] {
            let p = perms(|p| p.session_bus = strings(&[policy]));
            assert_eq!(broad_flags(&p), [true], "{policy}");
        }
    }

    #[test]
    fn narrow_access_is_not_broad() {
        let p = perms(|p| {
            p.filesystems = strings(&["xdg-download", "xdg-run/pipewire-0", "xdg-datafoo"]);
            p.devices = strings(&["dri"]);
            p.sockets = strings(&["wayland", "pulseaudio"]);
            // The sandboxed portal, unlike the Flatpak service itself, keeps spawned programs inside.
            p.session_bus = strings(&[
                "org.freedesktop.portal.Flatpak = talk",
                "org.freedesktop.Notifications = talk",
            ]);
        });
        assert!(broad_flags(&p).iter().all(|broad| !broad));
    }

    #[test]
    fn host_command_services_get_their_own_broad_row() {
        let p = perms(|p| {
            p.session_bus = strings(&[
                "org.freedesktop.Flatpak = talk",
                "org.freedesktop.Notifications = talk",
            ]);
        });
        let services = permission_groups(&p).pop().unwrap().1;
        assert_eq!(services.len(), 2);
        assert!(services[0].broad && services[0].detail.contains("org.freedesktop.Flatpak"));
        assert!(!services[1].broad && !services[1].detail.contains("org.freedesktop.Flatpak"));
    }

    #[test]
    fn fully_sandboxed_only_without_any_exception() {
        let sandboxed =
            crate::i18n::label("Runs fully sandboxed \u{2014} no extra access requested.");
        let revocations_only = perms(|p| {
            p.filesystems = strings(&["!home"]);
            p.session_bus = strings(&["org.example.Service = none"]);
        });
        for empty in [Permissions::default(), revocations_only] {
            assert_eq!(permission_glance(Some(&empty)), sandboxed);
            assert!(permission_badges(&permission_groups(&empty)).is_empty());
        }
        for exception in [
            perms(|p| p.devices = strings(&["all"])),
            perms(|p| p.sockets = strings(&["session-bus"])),
            perms(|p| p.session_bus = strings(&["org.freedesktop.Flatpak = talk"])),
            perms(|p| p.filesystems = strings(&["~/.ssh"])),
            perms(|p| p.sockets = strings(&["wayland"])),
        ] {
            assert_ne!(
                permission_glance(Some(&exception)),
                sandboxed,
                "{exception:?}"
            );
            assert!(!permission_badges(&permission_groups(&exception)).is_empty());
        }
    }

    #[test]
    fn every_broad_item_has_a_warning_badge() {
        let p = perms(|p| {
            p.shared = strings(&["network"]);
            p.sockets = strings(&["wayland", "x11"]);
            p.devices = strings(&["input", "dri"]);
            p.filesystems = strings(&["xdg-data/flatpak/overrides", "xdg-music"]);
            p.session_bus = strings(&["org.freedesktop.systemd1 = talk"]);
        });
        let groups = permission_groups(&p);
        let badges = permission_badges(&groups);
        for item in groups.iter().flat_map(|(_, items)| items) {
            assert_eq!(
                badges
                    .iter()
                    .any(|(label, broad)| *broad && *label == item.title),
                item.broad,
                "{}",
                item.title
            );
        }
    }
}
