import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
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
    const hiddenBrand = document.querySelector(".st-models-brand[hidden]");
    expect(hiddenBrand).not.toBeNull();
    expect(window.getComputedStyle(hiddenBrand as Element).display).toBe("none");

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
    await user.type(
      within(
        screen.getByRole("group", { name: "PaddleOCR (Baidu)" }),
      ).getByLabelText("API token"),
      "paddle-draft",
    );

    await user.selectOptions(picker, "mineru");
    const mineruGroup = screen.getByRole("group", {
      name: "MinerU Precision Extract",
    });
    const mineruToken = within(mineruGroup).getByLabelText(
      "API token",
    ) as HTMLInputElement;
    expect(mineruToken.value).toBe("");
    await user.type(mineruToken, "mineru-draft");

    await user.selectOptions(picker, "paddleocr");
    expect(
      (
        within(
          screen.getByRole("group", { name: "PaddleOCR (Baidu)" }),
        ).getByLabelText("API token") as HTMLInputElement
      ).value,
    ).toBe("paddle-draft");

    await user.selectOptions(picker, "mineru");
    expect(
      (within(mineruGroup).getByLabelText("API token") as HTMLInputElement).value,
    ).toBe("mineru-draft");
  });

  it("keeps each slot busy while its request is in flight", async () => {
    let finishSave = () => {};
    api.saveOcrCredential.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finishSave = resolve;
        }),
    );
    const user = userEvent.setup();
    render(<OcrSettingsPanel locale="en" />);

    const picker = await screen.findByRole("combobox", { name: "OCR model" });
    await user.type(
      within(
        screen.getByRole("group", { name: "PaddleOCR (Baidu)" }),
      ).getByLabelText("API token"),
      "paddle-token",
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.saveOcrCredential).toHaveBeenCalledTimes(1));
    expect(
      (screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    await user.selectOptions(picker, "mineru");
    await user.selectOptions(picker, "paddleocr");

    expect(
      (screen.getByRole("button", { name: "Saving…" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    finishSave();
    expect(await screen.findByText("Saved")).not.toBeNull();
  });
});
