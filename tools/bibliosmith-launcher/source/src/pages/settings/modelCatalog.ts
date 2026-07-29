// UI metadata for the model providers. The backend (get_model_catalog) is the
// source of truth for which slots exist and which have a stored key; this only
// adds display names, model preset lists and where to get a key — things the
// registry has no reason to carry. Keys here are (profileId, configId), matching
// the engine's providers.toml.

export type ModelSlotMeta = {
  profileId: string;
  configId: string;
  label: string;
  models: string[];
  keyUrl: string;
  allowCustomModel?: boolean;
};

export type ProviderBrand = {
  profileId: string;
  brand: string;
  keyUrl: string;
  slots: ModelSlotMeta[];
};

// One entry per brand. A brand with two supported billing routes (currently
// MiMo) lists two slots; the others list one. Order is roughly "simplest to set
// up" first.
export const MODEL_BRANDS: ProviderBrand[] = [
  {
    profileId: "deepseek",
    brand: "DeepSeek",
    keyUrl: "https://platform.deepseek.com/api_keys",
    slots: [
      {
        profileId: "deepseek",
        configId: "deepseek-default",
        label: "DeepSeek",
        models: ["deepseek-v4-flash", "deepseek-v4-pro", "deepseek-chat"],
        keyUrl: "https://platform.deepseek.com/api_keys",
      },
    ],
  },
  {
    profileId: "openai-compatible",
    brand: "OpenAI",
    keyUrl: "https://platform.openai.com/api-keys",
    slots: [
      {
        profileId: "openai-compatible",
        configId: "openai-default",
        label: "OpenAI",
        models: ["gpt-4.1-mini", "gpt-4.1", "gpt-4o-mini"],
        keyUrl: "https://platform.openai.com/api-keys",
      },
    ],
  },
  {
    profileId: "gemini-native",
    brand: "Gemini",
    keyUrl: "https://aistudio.google.com/apikey",
    slots: [
      {
        profileId: "gemini-native",
        configId: "gemini-default",
        label: "Gemini",
        models: ["gemini-2.5-flash", "gemini-2.5-pro"],
        keyUrl: "https://aistudio.google.com/apikey",
      },
    ],
  },
  {
    profileId: "kimi",
    brand: "Kimi · Moonshot",
    keyUrl: "https://platform.moonshot.ai/console/api-keys",
    slots: [
      {
        profileId: "kimi",
        configId: "kimi-default",
        label: "Kimi",
        models: ["kimi-k2.6"],
        keyUrl: "https://platform.moonshot.ai/console/api-keys",
      },
    ],
  },
  {
    profileId: "qwen",
    brand: "阿里云百炼 · Qwen",
    keyUrl: "https://bailian.console.aliyun.com/",
    slots: [
      {
        profileId: "qwen",
        configId: "payg",
        label: "中国大陆",
        models: ["qwen3.7-max", "qwen3.7-plus", "qwen3.6-plus", "qwen3.6-flash"],
        keyUrl: "https://bailian.console.aliyun.com/",
        allowCustomModel: true,
      },
    ],
  },
  {
    profileId: "doubao",
    brand: "火山方舟 · Doubao",
    keyUrl: "https://console.volcengine.com/ark/region:ark+cn-beijing/apikey",
    slots: [
      {
        profileId: "doubao",
        configId: "cn-beijing",
        label: "华北 2（北京）",
        models: [
          "doubao-seed-evolving",
          "doubao-seed-2-1-pro-260628",
          "doubao-seed-2-1-turbo-260628",
        ],
        keyUrl: "https://console.volcengine.com/ark/region:ark+cn-beijing/apikey",
        allowCustomModel: true,
      },
    ],
  },
  {
    profileId: "mimo",
    brand: "小米 MiMo",
    keyUrl: "https://xiaomimimo.com/",
    slots: [
      {
        profileId: "mimo",
        configId: "payg",
        label: "按量付费 · pay-as-you-go",
        models: ["mimo-v2.5"],
        keyUrl: "https://xiaomimimo.com/",
      },
      {
        profileId: "mimo",
        configId: "token-plan",
        label: "Token Plan",
        models: ["mimo-v2.5"],
        keyUrl: "https://xiaomimimo.com/",
      },
    ],
  },
];

export function slotMeta(
  profileId: string,
  configId: string,
): ModelSlotMeta | undefined {
  for (const brand of MODEL_BRANDS) {
    const hit = brand.slots.find(
      (slot) => slot.profileId === profileId && slot.configId === configId,
    );
    if (hit) return hit;
  }
  return undefined;
}

export function slotKey(profileId: string, configId: string): string {
  return `${profileId}:${configId}`;
}

// The picker label for a slot: the brand alone when it bills one way, brand plus
// plan when it bills two. Shared by every picker so they cannot drift. A slot the
// catalog does not list still gets a readable name rather than an empty option.
export function slotDisplayName(profileId: string, configId: string): string {
  for (const brand of MODEL_BRANDS) {
    const hit = brand.slots.find(
      (slot) => slot.profileId === profileId && slot.configId === configId,
    );
    if (hit) return brand.slots.length > 1 ? `${brand.brand} · ${hit.label}` : brand.brand;
  }
  return `${profileId} · ${configId}`;
}
