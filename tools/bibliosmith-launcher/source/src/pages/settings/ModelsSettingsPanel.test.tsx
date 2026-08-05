import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";

const api = vi.hoisted(() => ({
  deleteModelCredential: vi.fn(),
  getModelCatalog: vi.fn(),
  saveQwenSettings: vi.fn(),
  saveModelCredential: vi.fn(),
  setActiveModel: vi.fn(),
  testModelConnection: vi.fn(),
}));

vi.mock("../../api", () => api);

describe("ModelsSettingsPanel", () => {
  beforeEach(() => {
    for (const mock of Object.values(api)) mock.mockReset();
    api.getModelCatalog.mockResolvedValue({
      slots: [
        {
          profileId: "doubao",
          configId: "cn-beijing",
          providerType: "openai-responses",
          defaultModel: "doubao-seed-2-1-pro-260628",
          configured: true,
        },
        {
          profileId: "qwen",
          configId: "payg",
          providerType: "openai-responses",
          defaultModel: "qwen3.7-max",
          configured: true,
          workspaceId: null,
          webSearchEnabled: false,
        },
      ],
      active: null,
    });
    api.setActiveModel.mockResolvedValue(undefined);
    api.saveQwenSettings.mockResolvedValue(undefined);
  });

  it("shows one translation model configuration at a time and switches it from one picker", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    const picker = await screen.findByRole("combobox", {
      name: "Translation model",
    });
    expect(
      screen.getByRole("group", { name: "火山方舟 · Doubao" }),
    ).not.toBeNull();
    expect(
      screen.queryByRole("group", { name: "阿里云百炼 · Qwen" }),
    ).toBeNull();

    await user.selectOptions(picker, "qwen:payg");

    expect(
      screen.getByRole("group", { name: "阿里云百炼 · Qwen" }),
    ).not.toBeNull();
    expect(
      screen.queryByRole("group", { name: "火山方舟 · Doubao" }),
    ).toBeNull();
  });

  it("accepts current provider model or endpoint IDs instead of locking the user to stale presets", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    const picker = screen.getByRole("combobox", { name: "Translation model" });
    const brand = screen.getByRole("group", { name: "火山方舟 · Doubao" });
    const model = within(brand).getByLabelText("Model");
    expect(model.tagName).toBe("INPUT");
    expect(
      brand.querySelector('option[value="doubao-seed-evolving"]'),
    ).not.toBeNull();

    await user.clear(model);
    await user.type(model, "ep-book-translation");
    await user.click(within(brand).getByRole("button", { name: "Use this" }));

    await waitFor(() =>
      expect(api.setActiveModel).toHaveBeenCalledWith(
        "doubao",
        "cn-beijing",
        "ep-book-translation",
      ),
    );

    await user.selectOptions(picker, "qwen:payg");
    const qwenBrand = screen.getByRole("group", {
      name: "阿里云百炼 · Qwen",
    });
    expect(within(qwenBrand).getByLabelText("Model").tagName).toBe("INPUT");
  });

  it("keeps the Ark default model when the API key is entered first", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    const brand = screen.getByRole("group", {
      name: "火山方舟 · Doubao",
    });
    const model = within(brand).getByLabelText("Model") as HTMLInputElement;
    const key = within(brand).getByLabelText("API key");

    await user.type(key, "ark-key");

    expect(model.value).toBe("doubao-seed-2-1-pro-260628");
  });

  it("keeps provider slot drafts separate and only shows Qwen-specific fields", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    const picker = screen.getByRole("combobox", { name: "Translation model" });
    const doubaoModel = within(
      screen.getByRole("group", { name: "火山方舟 · Doubao" }),
    ).getByLabelText("Model") as HTMLInputElement;
    expect(screen.queryByLabelText("Workspace ID (optional)")).toBeNull();
    await user.clear(doubaoModel);
    await user.type(doubaoModel, "doubao-slot-draft");

    await user.selectOptions(picker, "qwen:payg");
    const qwenModel = within(
      screen.getByRole("group", { name: "阿里云百炼 · Qwen" }),
    ).getByLabelText("Model") as HTMLInputElement;
    expect(screen.getByLabelText("Workspace ID (optional)")).not.toBeNull();
    expect(qwenModel.value).toBe("qwen3.7-max");
    await user.clear(qwenModel);
    await user.type(qwenModel, "qwen-slot-draft");

    await user.selectOptions(picker, "doubao:cn-beijing");
    expect(
      (
        within(
          screen.getByRole("group", { name: "火山方舟 · Doubao" }),
        ).getByLabelText("Model") as HTMLInputElement
      ).value,
    ).toBe("doubao-slot-draft");
    expect(screen.queryByLabelText("Workspace ID (optional)")).toBeNull();

    await user.selectOptions(picker, "qwen:payg");
    expect(
      (
        within(
          screen.getByRole("group", { name: "阿里云百炼 · Qwen" }),
        ).getByLabelText("Model") as HTMLInputElement
      ).value,
    ).toBe("qwen-slot-draft");
  });

  it("stores Qwen Workspace ID and optional web search separately from the API key", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Translation model" }),
      "qwen:payg",
    );
    const brand = screen.getByRole("group", {
      name: "阿里云百炼 · Qwen",
    });
    const workspace = within(brand).getByLabelText("Workspace ID (optional)");
    const webSearch = within(brand).getByLabelText("Web search (optional)");

    await user.type(workspace, "ws-abc123");
    await user.click(webSearch);
    await user.click(within(brand).getByRole("button", { name: "Save Qwen settings" }));

    await waitFor(() =>
      expect(api.saveQwenSettings).toHaveBeenCalledWith(
        "ws-abc123",
        true,
      ),
    );
  });
});
