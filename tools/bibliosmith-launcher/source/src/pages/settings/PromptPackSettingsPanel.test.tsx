import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import catalogJson from "../../../src-tauri/resources/translation-prompt-packs.json";
import type { TranslationPromptPackCatalog } from "../../types";
import { PromptPackSettingsPanel, type PromptPackSettingsPanelProps } from "./PromptPackSettingsPanel";

const catalog = catalogJson as TranslationPromptPackCatalog;
const structure = catalog.packs[0].revisions[0];

function props(over: Partial<PromptPackSettingsPanelProps> = {}): PromptPackSettingsPanelProps {
  return {
    locale: "zh-CN",
    catalog,
    defaults: {
      programmatic: {
        packId: structure.packId,
        revisionId: structure.revisionId,
        contentSha256: structure.contentSha256,
      },
      "expert-agent": null,
    },
    busy: false,
    onCopy: vi.fn(async () => undefined),
    onSaveRevision: vi.fn(async () => undefined),
    onDelete: vi.fn(async () => undefined),
    onSetDefault: vi.fn(async () => undefined),
    ...over,
  };
}

describe("PromptPackSettingsPanel", () => {
  it("shows the four functionally named built-ins and locks executor safety", () => {
    render(<PromptPackSettingsPanel {...props()} />);

    for (const name of ["结构保真翻译", "四维反思精修", "语境回溯精译", "全流程审校闭环"]) {
      expect(screen.getByRole("button", { name: new RegExp(name) })).toBeTruthy();
    }
    expect(screen.getByText("执行器安全层（不可编辑）")).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "方案名称" }).getAttribute("readonly")).not.toBeNull();
    expect((screen.getByRole("button", { name: /复制后编辑/ }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("shows immutable revisions and an offline stage-level diff without changing the default", async () => {
    const user = userEvent.setup();
    const onSetDefault = vi.fn(async () => undefined);
    render(<PromptPackSettingsPanel {...props({ onSetDefault })} />);
    await user.click(screen.getByRole("button", { name: /四维反思精修/ }));

    expect((screen.getByRole("combobox", { name: "查看修订" }) as HTMLSelectElement).value).toBe("2026.08.05-2");
    await user.click(screen.getByRole("button", { name: "与上一修订比较" }));

    const diff = await screen.findByLabelText("修订差异");
    expect(within(diff).getByText("2026.08.05-1 → 2026.08.05-2")).toBeTruthy();
    expect(within(diff).getByText("reflect")).toBeTruthy();
    expect(within(diff).getByText("adaptation")).toBeTruthy();
    expect(onSetDefault).not.toHaveBeenCalled();
    expect((screen.getByRole("button", { name: "设为该执行方式的默认方案" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("saves edits as a new local revision instead of mutating a built-in", async () => {
    const localPack = structuredClone(catalog.packs[0]);
    localPack.packId = "local.test";
    localPack.kind = "custom";
    localPack.revisions[0].packId = "local.test";
    localPack.revisions[0].displayName = "我的结构译法";
    const localCatalog = { ...catalog, packs: [...catalog.packs, localPack] };
    const onSaveRevision = vi.fn(async () => undefined);
    const user = userEvent.setup();
    render(<PromptPackSettingsPanel {...props({ catalog: localCatalog, onSaveRevision })} />);
    await user.click(screen.getByRole("button", { name: /我的结构译法/ }));

    const template = screen.getByRole("textbox", { name: "结构保真初译 模板" });
    const styleGuidance = screen.getByRole("textbox", { name: "风格指导参数" });
    await user.clear(template);
    await user.type(template, "按本书语体完整翻译当前块。");
    await user.type(styleGuidance, "克制的现代汉语");
    await user.click(screen.getByRole("button", { name: "保存为新版本" }));

    expect(onSaveRevision).toHaveBeenCalledWith(expect.objectContaining({
      packId: "local.test",
      parameters: { styleGuidance: "克制的现代汉语" },
      stages: [expect.objectContaining({ template: "按本书语体完整翻译当前块。" })],
    }));
  });

  it("shows pinned expert skills, mechanism references, and excluded responsibilities", async () => {
    const user = userEvent.setup();
    render(<PromptPackSettingsPanel {...props()} />);

    await user.click(screen.getByRole("button", { name: /语境回溯精译/ }));
    expect(screen.getByText("固定技能依赖")).toBeTruthy();
    expect(screen.getByText(/expert-translation-quality@sha256:b97f2eaa/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /全流程审校闭环/ }));
    expect(screen.getByText("机制参考文件")).toBeTruthy();
    expect(screen.getByText(/book-runner\.md/)).toBeTruthy();
    expect(screen.getByText("许可证 / 使用边界").parentElement?.textContent).toContain("CC BY-NC-SA");
    expect(screen.getByText("明确排除的职责")).toBeTruthy();
    expect(screen.getByText(/copyright-decision/)).toBeTruthy();
  });
});
