// History & Versions page (/ui/history): locate an instance, pick two versions,
// see the two-layer diff, toggle metadata churn on/off.
import type { Page, Locator } from "@playwright/test";

export class HistoryPage {
  constructor(readonly page: Page) {}

  async goto(deepLink?: { type: string; id: string }): Promise<void> {
    const q = deepLink ? `?type=${deepLink.type}&id=${deepLink.id}` : "";
    await this.page.goto(`/ui/history${q}`, { waitUntil: "networkidle" });
  }

  get locateForm(): Locator {
    return this.page.locator("#history-locate");
  }
  get subject(): Locator {
    return this.page.locator("#history-subject");
  }
  get controls(): Locator {
    return this.page.locator("#history-controls");
  }
  get versions(): Locator {
    return this.page.locator("#history-versions .history-version");
  }
  get fromSelect(): Locator {
    return this.page.locator("#history-from");
  }
  get toSelect(): Locator {
    return this.page.locator("#history-to");
  }
  get metadataCheckbox(): Locator {
    return this.page.locator("#history-show-metadata");
  }
  get diff(): Locator {
    return this.page.locator("#history-diff");
  }
  // The field-level "what changed" layer, as opposed to the textual JSON layer.
  get semanticDiff(): Locator {
    return this.page.locator("#history-diff .diff__semantic");
  }
  get textualDetails(): Locator {
    return this.page.locator("#history-diff details.diff__textual");
  }

  tab(name: "instance" | "type" | "system"): Locator {
    return this.page.locator(`[data-tab='${name}']`);
  }

  /** Type a type/id into the locate form and submit. */
  async locate(type: string, id: string): Promise<void> {
    await this.locateForm.locator("[name=type]").fill(type);
    await this.locateForm.locator("[name=id]").fill(id);
    await this.locateForm.locator("button[type=submit]").click();
  }
}
