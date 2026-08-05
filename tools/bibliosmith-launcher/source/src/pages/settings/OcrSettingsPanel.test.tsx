import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OcrSettingsPanel } from "./OcrSettingsPanel";

const api = vi.hoisted(() => ({
  deleteOcrCredential: vi.fn(),
  getOcrCredentialsStatus: vi.fn(),
  saveOcrCredential: vi.fn(),
  testOcrConnection: vi.fn(),
}));

vi.mock("../../api", () => api);

describe("OcrSettingsPanel", () => {
  beforeEach(() => {
    for (const mock of Object.values(api)) mock.mockReset();
    api.getOcrCredentialsStatus.mockResolvedValue({
      paddleocr: { configured: true, source: "keychain" },
      mineru: { configured: true, source: "keychain" },
    });
  });

  it("shows one OCR model configuration at a time and switches it from one picker", async () => {
    const user = userEvent.setup();
    render(<OcrSettingsPanel locale="en" />);

    const picker = await screen.findByRole("combobox", { name: "OCR model" });
    expect(
      screen.getByRole("group", { name: "PaddleOCR (Baidu)" }),
    ).not.toBeNull();
    expect(
      screen.queryByRole("group", { name: "MinerU Precision Extract" }),
    ).toBeNull();

    await user.selectOptions(picker, "mineru");

    expect(
      screen.getByRole("group", { name: "MinerU Precision Extract" }),
    ).not.toBeNull();
    expect(
      screen.queryByRole("group", { name: "PaddleOCR (Baidu)" }),
    ).toBeNull();
  });

  it("keeps a separate unsaved token draft for each OCR slot", async () => {
    const user = userEvent.setup();
    render(<OcrSettingsPanel locale="en" />);

    const picker = await screen.findByRole("combobox", { name: "OCR model" });
    await user.type(screen.getByLabelText("API token"), "paddle-draft");

    await user.selectOptions(picker, "mineru");
    const mineruToken = screen.getByLabelText("API token") as HTMLInputElement;
    expect(mineruToken.value).toBe("");
    await user.type(mineruToken, "mineru-draft");

    await user.selectOptions(picker, "paddleocr");
    expect((screen.getByLabelText("API token") as HTMLInputElement).value).toBe(
      "paddle-draft",
    );

    await user.selectOptions(picker, "mineru");
    expect((screen.getByLabelText("API token") as HTMLInputElement).value).toBe(
      "mineru-draft",
    );
  });
});
