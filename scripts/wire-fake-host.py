"""Insert the fake_host_domain! invocation and the HostOsControl port accessors."""

import pathlib
import re

path = pathlib.Path("/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/testing.rs")
src = path.read_text(encoding="utf-8")

DOMAINS = [
    ("with_audio", "audio", "crate::os_control::audio::AudioControlPort"),
    ("with_power", "power", "crate::os_control::power::PowerControlPort"),
    (
        "with_power_session",
        "power_session",
        "crate::os_control::power::session::PowerSessionControlPort",
    ),
    ("with_processes", "processes", "crate::os_control::processes::ProcessControlPort"),
    ("with_display", "display", "crate::os_control::display::DisplayControlPort"),
    (
        "with_connectivity",
        "connectivity",
        "crate::os_control::connectivity::ConnectivityControlPort",
    ),
    ("with_clipboard", "clipboard", "crate::os_control::clipboard::ClipboardControlPort"),
    (
        "with_notifications",
        "notifications",
        "crate::os_control::notifications::NotificationControlPort",
    ),
    ("with_packages", "packages", "crate::os_control::packages::PackageControlPort"),
    ("with_storage", "storage", "crate::os_control::storage::StorageControlPort"),
    ("with_trash", "trash", "crate::os_control::files::TrashControlPort"),
    (
        "with_application_close",
        "application_close",
        "crate::os_control::applications::ApplicationCloseControlPort",
    ),
    (
        "with_desktop_association",
        "desktop_association",
        "crate::os_control::applications::DesktopAssociationControlPort",
    ),
]

# 1. The builder macro takes builder-name -> field, so emit an explicit impl block
#    instead (clearer than a macro that has to map two names).
builders = ["impl FakeHostOsControl {"]
for builder, field, port in DOMAINS:
    builders.append(f"    /// Builder: compose the `{field}` port into the aggregate.")
    builders.append("    #[must_use]")
    builders.append(f"    pub fn {builder}(mut self, port: Arc<dyn {port}>) -> Self {{")
    builders.append(f"        self.{field} = Some(port);")
    builders.append("        self")
    builders.append("    }")
    builders.append("")
builders.append("}")
builder_block = "\n".join(builders)

# Replace the macro definition with the explicit impl block.
macro_start = src.index("/// Generate a `with_<domain>` builder plus the `HostOsControl` accessor")
macro_end = src.index("impl FakeHostOsControl {", macro_start)
src = src[:macro_start] + src[macro_end:]

# Insert the builder impl right before `impl HostOsControl for FakeHostOsControl {`
anchor = "impl HostOsControl for FakeHostOsControl {"
src = src.replace(anchor, builder_block + "\n\n" + anchor, 1)

# 2. Port accessors inside the HostOsControl impl, after provider_id.
accessors = []
for _builder, field, port in DOMAINS:
    accessors.append(f"    fn {field}(&self) -> Option<&dyn {port}> {{")
    accessors.append(f'        self.recorder.record("{field}");')
    accessors.append(f"        self.{field}.as_deref()")
    accessors.append("    }")
    accessors.append("")
accessor_block = "\n".join(accessors)

marker = """        self.recorder.record("provider_id");
        ProviderId::new(&self.provider)
    }"""
src = src.replace(marker, marker + "\n\n" + accessor_block, 1)

path.write_text(src, encoding="utf-8")
print("inserted 13 builders and 13 accessors")
