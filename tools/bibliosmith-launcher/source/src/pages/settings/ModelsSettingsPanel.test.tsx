import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";

const api = vi.hoisted(() => ({
  deleteModelCredential: vi.fn(),
  getModelCatalog: vi.fn(),
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
          providerType: "openai-compatible",
          defaultModel: "doubao-seed-2-1-pro-260628",
          configured: true,
        },
      ],
      active: null,
    });
    api.setActiveModel.mockResolvedValue(undefined);
  });

  it("accepts current provider model or endpoint IDs instead of locking the user to stale presets", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    const heading = screen
      .getAllByText("火山方舟 · Doubao")
      .find((node) => node.tagName === "B");
    expect(heading).toBeDefined();
    const brand = heading!.closest(".st-models-brand");
    expect(brand).not.toBeNull();
    const model = within(brand as HTMLElement).getByLabelText("Model");
    expect(model.tagName).toBe("INPUT");
    expect(
      brand!.querySelector('option[value="doubao-seed-evolving"]'),
    ).not.toBeNull();

    const qwenHeading = screen
      .getAllByText("阿里云百炼 · Qwen")
      .find((node) => node.tagName === "B");
    const qwenBrand = qwenHeading!.closest(".st-models-brand");
    expect(within(qwenBrand as HTMLElement).getByLabelText("Model").tagName).toBe(
      "INPUT",
    );

    await user.clear(model);
    await user.type(model, "ep-book-translation");
    await user.click(within(brand as HTMLElement).getByRole("button", { name: "Use this" }));

    await waitFor(() =>
      expect(api.setActiveModel).toHaveBeenCalledWith(
        "doubao",
        "cn-beijing",
        "ep-book-translation",
      ),
    );
  });

  it("keeps the Ark default model when the API key is entered first", async () => {
    const user = userEvent.setup();
    render(<ModelsSettingsPanel locale="en" />);

    await waitFor(() => expect(api.getModelCatalog).toHaveBeenCalled());
    const heading = screen
      .getAllByText("火山方舟 · Doubao")
      .find((node) => node.tagName === "B");
    const brand = heading!.closest(".st-models-brand") as HTMLElement;
    const model = within(brand).getByLabelText("Model") as HTMLInputElement;
    const key = within(brand).getByLabelText("API key");

    await user.type(key, "ark-key");

    expect(model.value).toBe("doubao-seed-2-1-pro-260628");
  });
});
