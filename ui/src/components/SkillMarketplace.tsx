import { Component, createSignal, For, Show, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { RemoteInstallRequest, RemoteSkillCard, SkillDescriptor } from "../types/openclaw";

// ── Trust tier colour map ─────────────────────────────────────────────────────
const TRUST_COLORS: Record<string, { bg: string; text: string; label: string }> = {
  verified:  { bg: "#dcfce7", text: "#166534", label: "Verified" },
  community: { bg: "#fef3c7", text: "#92400e", label: "Community" },
  local:     { bg: "#f3f4f6", text: "#374151", label: "Local" },
};

function trustStyle(tier: string) {
  return TRUST_COLORS[tier.toLowerCase()] ?? TRUST_COLORS.local;
}

// ── Capability badge helpers ──────────────────────────────────────────────────
interface Badges { icon: string; label: string }
function capabilityBadges(skill: SkillDescriptor): Badges[] {
  const b: Badges[] = [];
  if (skill.enabled) {
    // Infer from trust/category since SkillCard doesn't carry full capabilities
    const cat = skill.category.toLowerCase();
    if (cat === "web" || skill.slug.includes("web") || skill.slug.includes("search")) {
      b.push({ icon: "🌐", label: "Network" });
    }
    if (cat === "productivity" || skill.slug.includes("calc")) {
      b.push({ icon: "🧮", label: "Compute" });
    }
    if (skill.slug.includes("file") || skill.slug.includes("disk")) {
      b.push({ icon: "📁", label: "Filesystem" });
    }
    if (skill.slug.includes("browser") || skill.slug.includes("desktop")) {
      b.push({ icon: "🖥️", label: "Desktop" });
    }
  }
  return b;
}

// ── PermissionModal ───────────────────────────────────────────────────────────
interface PermissionModalProps {
  skill: RemoteSkillCard;
  onApprove: (req: RemoteInstallRequest) => void;
  onDismiss: () => void;
}

const PermissionModal: Component<PermissionModalProps> = (props) => {
  const trust = trustStyle(props.skill.trust_tier);
  return (
    <div style={{
      position: "fixed", inset: 0, background: "rgba(0,0,0,0.45)",
      display: "flex", "align-items": "center", "justify-content": "center",
      "z-index": "1000",
    }}>
      <div style={{
        background: "#fff", "border-radius": "16px", padding: "24px",
        width: "420px", "max-width": "90vw", "box-shadow": "0 8px 40px rgba(0,0,0,0.18)",
        display: "flex", "flex-direction": "column", gap: "14px",
      }}>
        <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
          <h3 style={{ margin: 0, "font-size": "16px", "font-weight": "700" }}>Install Skill</h3>
          <span style={{
            "font-size": "11px", "font-weight": "600", padding: "2px 8px",
            "border-radius": "999px", background: trust.bg, color: trust.text,
          }}>{trust.label}</span>
        </div>

        <div>
          <div style={{ "font-weight": "600", "font-size": "14px" }}>{props.skill.name}</div>
          <div style={{ "font-size": "11px", color: "#6b7280", "margin-top": "2px" }}>
            v{props.skill.version} · {props.skill.category}
          </div>
        </div>

        <p style={{ margin: 0, "font-size": "13px", color: "#374151", "line-height": "1.5" }}>
          {props.skill.description}
        </p>

        {/* Capability summary */}
        <div style={{ background: "#f9fafb", "border-radius": "8px", padding: "12px" }}>
          <div style={{ "font-size": "11px", "font-weight": "600", color: "#374151", "margin-bottom": "6px" }}>
            Requested Capabilities
          </div>
          <Show
            when={props.skill.capabilities_summary.length > 0}
            fallback={
              <span style={{ "font-size": "12px", color: "#22c55e" }}>✓ No special permissions required</span>
            }
          >
            <For each={props.skill.capabilities_summary}>{(cap) => (
              <div style={{ "font-size": "12px", color: "#4b5563", padding: "2px 0" }}>
                ⚠ {cap}
              </div>
            )}</For>
          </Show>
        </div>

        <div style={{ "font-size": "11px", color: "#9ca3af" }}>
          Source: <code style={{ "font-size": "10px" }}>{props.skill.manifest_url}</code>
        </div>

        <div style={{ display: "flex", gap: "8px", "justify-content": "flex-end" }}>
          <button
            style={btnStyle("#f3f4f6", "#374151")}
            onClick={props.onDismiss}
          >Cancel</button>
          <button
            style={btnStyle("#6366f1", "#ffffff")}
            onClick={() => props.onApprove({
              manifest_url: props.skill.manifest_url,
              slug: props.skill.slug,
              approved_capabilities: {},
            })}
          >Approve & Install</button>
        </div>
      </div>
    </div>
  );
};

// ── Main component ────────────────────────────────────────────────────────────
const SkillMarketplace: Component = () => {
  // Local tab state
  const [localSkills, setLocalSkills] = createSignal<SkillDescriptor[]>([]);
  const [localQuery, setLocalQuery] = createSignal("");
  const [localCategory, setLocalCategory] = createSignal<string | null>(null);
  const [localLoading, setLocalLoading] = createSignal(false);

  // Remote/browse tab state
  const [remoteSkills, setRemoteSkills] = createSignal<RemoteSkillCard[]>([]);
  const [remoteQuery, setRemoteQuery] = createSignal("");
  const [remoteLoading, setRemoteLoading] = createSignal(false);
  const [remoteFetched, setRemoteFetched] = createSignal(false);

  // Shared state
  const [activeTab, setActiveTab] = createSignal<"all" | "installed" | "browse">("all");
  const [localError, setLocalError] = createSignal<string | null>(null);
  const [remoteError, setRemoteError] = createSignal<string | null>(null);
  const [pendingInstall, setPendingInstall] = createSignal<RemoteSkillCard | null>(null);
  const [installing, setInstalling] = createSignal<string | null>(null);

  // ── Local fetch ──────────────────────────────────────────────────────────
  const fetchLocal = async () => {
    setLocalLoading(true);
    setLocalError(null);
    try {
      const result = await invoke<SkillDescriptor[]>("clawhub_search_skills", {
        query: localQuery(),
        category: localCategory(),
        limit: null,
      });
      setLocalSkills(result);
    } catch (e: unknown) {
      setLocalError(String(e));
    } finally {
      setLocalLoading(false);
    }
  };

  // ── Remote fetch ─────────────────────────────────────────────────────────
  const fetchRemote = async () => {
    setRemoteLoading(true);
    setRemoteError(null);
    try {
      const result = await invoke<RemoteSkillCard[]>("clawhub_fetch_remote_skills", {
        query: remoteQuery(),
        category: null,
      });
      setRemoteSkills(result);
      setRemoteFetched(true);
    } catch (e: unknown) {
      const msg = String(e);
      // Friendly message when the registry repo hasn't been created yet
      if (msg.includes("404") || msg.includes("HTTP 404")) {
        setRemoteError("Registry not available yet. Create your index.json at the configured GitHub URL to enable the marketplace.");
      } else {
        setRemoteError(msg);
      }
      setRemoteFetched(true);
    } finally {
      setRemoteLoading(false);
    }
  };

  onMount(() => { void fetchLocal(); });

  // Fetch remote when Browse tab is first activated
  const handleTabSwitch = (tab: "all" | "installed" | "browse") => {
    setActiveTab(tab);
    if (tab === "browse" && !remoteFetched()) void fetchRemote();
  };

  // ── Local actions ────────────────────────────────────────────────────────
  const handleToggle = async (skill: SkillDescriptor) => {
    setLocalError(null);
    try {
      await invoke("clawhub_toggle_skill", { skillId: skill.slug, enabled: !skill.enabled });
      await fetchLocal();
    } catch (e: unknown) { setLocalError(String(e)); }
  };

  const handleUninstall = async (skill: SkillDescriptor) => {
    setLocalError(null);
    try {
      await invoke("clawhub_uninstall_skill", { skillId: skill.slug });
      await fetchLocal();
    } catch (e: unknown) { setLocalError(String(e)); }
  };

  // ── Remote install ───────────────────────────────────────────────────────
  const handleApproveInstall = async (req: RemoteInstallRequest) => {
    setPendingInstall(null);
    setInstalling(req.slug);
    setRemoteError(null);
    try {
      await invoke("clawhub_install_skill", { request: req });
      // Refresh both lists
      await Promise.all([fetchLocal(), fetchRemote()]);
    } catch (e: unknown) {
      setRemoteError(String(e));
    } finally {
      setInstalling(null);
    }
  };

  const localCategories = () => [...new Set(localSkills().map((s) => s.category))].sort();
  const displayedLocal = () => {
    let list = localSkills();
    if (activeTab() === "installed") list = list.filter((s) => s.installed);
    return list;
  };

  return (
    <div style={{ padding: "16px", "font-family": "system-ui, sans-serif" }}>
      {/* Permission modal overlay */}
      <Show when={pendingInstall()}>
        <PermissionModal
          skill={pendingInstall()!}
          onApprove={handleApproveInstall}
          onDismiss={() => setPendingInstall(null)}
        />
      </Show>

      {/* Header */}
      <div style={{ "margin-bottom": "16px" }}>
        <h2 style={{ margin: "0 0 4px", "font-size": "20px", "font-weight": "700" }}>
          Skill Marketplace
        </h2>
        <p style={{ margin: 0, color: "#6b7280", "font-size": "13px" }}>
          Browse and manage installed skills.
        </p>
      </div>

      {/* Top-level tabs */}
      <div style={{ display: "flex", gap: "4px", "margin-bottom": "16px", "border-bottom": "1px solid #e5e7eb" }}>
        {(["all", "installed", "browse"] as const).map((tab) => (
          <button
            style={{
              padding: "6px 14px", border: "none", background: "transparent",
              "border-bottom": activeTab() === tab ? "2px solid #6366f1" : "2px solid transparent",
              color: activeTab() === tab ? "#6366f1" : "#6b7280",
              "font-size": "13px", cursor: "pointer",
              "font-weight": activeTab() === tab ? "600" : "400",
            }}
            onClick={() => handleTabSwitch(tab)}
          >
            {tab === "all" ? "All Skills" : tab === "installed" ? "Installed" : "🌐 Browse"}
          </button>
        ))}
      </div>

      {/* ── LOCAL TABS (All / Installed) ─────────────────────────────────── */}
      <Show when={activeTab() !== "browse"}>
        <Show when={localError()}>
          <div style={{
            background: "#fef2f2", color: "#991b1b", padding: "8px 12px",
            "border-radius": "8px", "font-size": "12px", "margin-bottom": "12px",
          }}>
            {localError()}
          </div>
        </Show>
        {/* Search + category filter row */}
        <div style={{ display: "flex", gap: "8px", "margin-bottom": "12px", "flex-wrap": "wrap" }}>
          <input
            type="search" placeholder="Search skills…" value={localQuery()}
            onInput={(e) => { setLocalQuery(e.currentTarget.value); void fetchLocal(); }}
            style={{
              flex: "1", "min-width": "180px", padding: "7px 12px",
              border: "1px solid #d1d5db", "border-radius": "8px", "font-size": "13px", outline: "none",
            }}
          />
          <button style={chipStyle(localCategory() === null)}
            onClick={() => { setLocalCategory(null); void fetchLocal(); }}>All</button>
          <For each={localCategories()}>{(cat) => (
            <button style={chipStyle(localCategory() === cat)}
              onClick={() => { setLocalCategory(cat); void fetchLocal(); }}>{cat}</button>
          )}</For>
        </div>

        <Show when={localLoading()}>
          <p style={{ color: "#9ca3af", "font-size": "13px" }}>Loading…</p>
        </Show>

        <Show when={!localLoading()}>
          <SkillGrid skills={displayedLocal()} onToggle={handleToggle} onUninstall={handleUninstall} />
          <Show when={displayedLocal().length === 0}>
            <EmptyState text="No skills found." />
          </Show>
        </Show>
      </Show>

      {/* ── BROWSE TAB (Remote registry) ────────────────────────────────── */}
      <Show when={activeTab() === "browse"}>
        <div style={{ display: "flex", gap: "8px", "margin-bottom": "12px" }}>
          <input
            type="search" placeholder="Search remote skills…" value={remoteQuery()}
            onInput={(e) => { setRemoteQuery(e.currentTarget.value); void fetchRemote(); }}
            style={{
              flex: "1", padding: "7px 12px", border: "1px solid #d1d5db",
              "border-radius": "8px", "font-size": "13px", outline: "none",
            }}
          />
          <button style={btnStyle("#6366f1", "#fff")} onClick={() => void fetchRemote()}>
            Refresh
          </button>
        </div>

        <Show when={remoteLoading()}>
          <p style={{ color: "#9ca3af", "font-size": "13px" }}>Fetching from registry…</p>
        </Show>

        <Show when={remoteError()}>
          <div style={{
            background: "#fffbeb", color: "#92400e", padding: "12px 14px",
            "border-radius": "8px", "font-size": "12px", "margin-bottom": "12px",
            border: "1px solid #fde68a",
          }}>
            ⚠ {remoteError()}
          </div>
        </Show>

        <Show when={!remoteLoading()}>
          <div style={{
            display: "grid",
            "grid-template-columns": "repeat(auto-fill, minmax(260px, 1fr))",
            gap: "12px",
          }}>
            <For each={remoteSkills()}>{(skill) => {
              const trust = trustStyle(skill.trust_tier);
              const isInstalling = () => installing() === skill.slug;
              return (
                <div style={{
                  background: "#fff", border: "1px solid #e5e7eb", "border-radius": "12px",
                  padding: "16px", display: "flex", "flex-direction": "column", gap: "10px",
                  "box-shadow": "0 1px 3px rgba(0,0,0,0.07)",
                }}>
                  <div style={{ display: "flex", "justify-content": "space-between", "align-items": "flex-start" }}>
                    <div>
                      <div style={{ "font-weight": "600", "font-size": "14px", "margin-bottom": "2px" }}>
                        {skill.name}
                      </div>
                      <span style={{
                        "font-size": "11px", padding: "2px 8px", "border-radius": "999px",
                        background: "#f3f4f6", color: "#6b7280",
                      }}>{skill.category}</span>
                    </div>
                    <div style={{ display: "flex", "flex-direction": "column", "align-items": "flex-end", gap: "4px" }}>
                      <span style={{
                        "font-size": "11px", "font-weight": "600", padding: "2px 8px",
                        "border-radius": "999px", background: trust.bg, color: trust.text,
                      }}>{trust.label}</span>
                      <span style={{ "font-size": "10px", color: "#9ca3af" }}>v{skill.version}</span>
                    </div>
                  </div>

                  <p style={{ margin: 0, "font-size": "12px", color: "#4b5563", "line-height": "1.5" }}>
                    {skill.description}
                  </p>

                  <Show when={skill.capabilities_summary.length > 0}>
                    <div style={{ display: "flex", gap: "4px", "flex-wrap": "wrap" }}>
                      <For each={skill.capabilities_summary}>{(cap) => (
                        <span style={{
                          "font-size": "10px", padding: "1px 6px", "border-radius": "999px",
                          background: "#fef3c7", color: "#92400e",
                        }}>⚠ {cap}</span>
                      )}</For>
                    </div>
                  </Show>

                  <div style={{ "margin-top": "auto" }}>
                    <Show
                      when={!skill.installed}
                      fallback={
                        <span style={{ "font-size": "12px", color: "#22c55e", "font-weight": "600" }}>
                          ✓ Installed
                        </span>
                      }
                    >
                      <button
                        style={btnStyle(isInstalling() ? "#e5e7eb" : "#6366f1", isInstalling() ? "#9ca3af" : "#fff")}
                        disabled={isInstalling()}
                        onClick={() => setPendingInstall(skill)}
                      >
                        {isInstalling() ? "Installing…" : "Install"}
                      </button>
                    </Show>
                  </div>
                </div>
              );
            }}</For>
          </div>

          <Show when={remoteSkills().length === 0 && remoteFetched()}>
            <EmptyState text="No remote skills found. The registry may be unavailable." />
          </Show>
        </Show>
      </Show>
    </div>
  );
};

// ── Style helpers ─────────────────────────────────────────────────────────────
function tabStyle(active: boolean): Record<string, string> {
  return {
    padding: "5px 12px", border: "1px solid", "border-radius": "999px",
    "font-size": "12px", cursor: "pointer", transition: "all 0.15s",
    background: active ? "#6366f1" : "#f9fafb",
    "border-color": active ? "#6366f1" : "#d1d5db",
    color: active ? "#ffffff" : "#374151",
  };
}

function chipStyle(active: boolean): Record<string, string> {
  return tabStyle(active);
}

function btnStyle(bg: string, color: string): Record<string, string> {
  return {
    flex: "1", padding: "6px 10px", border: "none", "border-radius": "8px",
    "font-size": "12px", "font-weight": "500", cursor: "pointer",
    background: bg, color,
  };
}

// ── SkillGrid sub-component (local installed skills) ──────────────────────────
interface SkillGridProps {
  skills: SkillDescriptor[];
  onToggle: (s: SkillDescriptor) => void;
  onUninstall: (s: SkillDescriptor) => void;
}

const SkillGrid: Component<SkillGridProps> = (props) => (
  <div style={{
    display: "grid",
    "grid-template-columns": "repeat(auto-fill, minmax(260px, 1fr))",
    gap: "12px",
  }}>
    <For each={props.skills}>{(skill) => {
      const trust = trustStyle(skill.trust_tier);
      const badges = capabilityBadges(skill);
      return (
        <div style={{
          background: "#ffffff", border: "1px solid #e5e7eb",
          "border-radius": "12px", padding: "16px",
          display: "flex", "flex-direction": "column", gap: "10px",
          "box-shadow": "0 1px 3px rgba(0,0,0,0.07)",
          opacity: skill.installed && !skill.enabled ? "0.65" : "1",
        }}>
          <div style={{ display: "flex", "justify-content": "space-between", "align-items": "flex-start" }}>
            <div>
              <div style={{ "font-weight": "600", "font-size": "14px", "margin-bottom": "2px" }}>
                {skill.name}
              </div>
              <span style={{
                "font-size": "11px", padding: "2px 8px", "border-radius": "999px",
                background: "#f3f4f6", color: "#6b7280",
              }}>{skill.category}</span>
            </div>
            <span style={{
              "font-size": "11px", "font-weight": "600", padding: "2px 8px",
              "border-radius": "999px", background: trust.bg, color: trust.text,
            }}>{trust.label}</span>
          </div>

          <p style={{ margin: 0, "font-size": "12px", color: "#4b5563", "line-height": "1.5" }}>
            {skill.description}
          </p>

          <Show when={badges.length > 0}>
            <div style={{ display: "flex", gap: "6px", "flex-wrap": "wrap" }}>
              <For each={badges}>{(b) => (
                <span style={{
                  "font-size": "11px", padding: "2px 7px", "border-radius": "999px",
                  background: "#eff6ff", color: "#1d4ed8",
                  display: "flex", "align-items": "center", gap: "3px",
                }}>{b.icon} {b.label}</span>
              )}</For>
            </div>
          </Show>

          <div style={{ display: "flex", gap: "6px", "margin-top": "auto" }}>
            <button
              style={btnStyle(skill.enabled ? "#f3f4f6" : "#dcfce7", skill.enabled ? "#374151" : "#166534")}
              onClick={() => props.onToggle(skill)}
            >{skill.enabled ? "Disable" : "Enable"}</button>
            <button
              style={btnStyle("#fef2f2", "#991b1b")}
              onClick={() => props.onUninstall(skill)}
            >Uninstall</button>
          </div>
        </div>
      );
    }}</For>
  </div>
);

// ── EmptyState ────────────────────────────────────────────────────────────────
const EmptyState: Component<{ text: string }> = (props) => (
  <p style={{ color: "#9ca3af", "font-size": "13px", "text-align": "center", padding: "32px 0" }}>
    {props.text}
  </p>
);

export default SkillMarketplace;
