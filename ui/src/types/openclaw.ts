/** Mirrors the SkillCard struct from kria-desktop commands/openclaw.rs */
export interface SkillDescriptor {
  slug: string;
  name: string;
  description: string;
  category: string;
  trust_tier: string;
  installed: boolean;
  enabled: boolean;
}

/** Mirrors SkillCapabilities from kria-core/src/openclaw/types.rs */
export interface SkillCapabilities {
  filesystem_read: boolean;
  filesystem_write: boolean;
  subprocess: boolean;
  browser: boolean;
  network: boolean;
  network_domains: string[];
  image_generation: boolean;
  media: boolean;
}

/** Mirrors ResourceProfile from kria-core/src/openclaw/types.rs */
export interface ResourceProfile {
  memory_limit: string;
  cpu_limit: string;
  timeout_secs: number;
  max_output_bytes: number;
  requires_approval: boolean;
  resource_class: string;
}

/** Mirrors SubstrateStatusPayload from kria-desktop commands/openclaw.rs */
export interface SubstrateStatus {
  status: string;
  details: string;
  active_invocations: number;
  warm_pool_count: number;
}

/** Mirrors RemoteSkillCard from kria-desktop commands/openclaw.rs */
export interface RemoteSkillCard {
  slug: string;
  name: string;
  description: string;
  category: string;
  trust_tier: string;
  version: string;
  manifest_url: string;
  capabilities_summary: string[];
  installed: boolean;
}

/** Payload sent to clawhub_install_skill */
export interface RemoteInstallRequest {
  manifest_url: string;
  slug: string;
  approved_capabilities?: Partial<SkillCapabilities>;
}
