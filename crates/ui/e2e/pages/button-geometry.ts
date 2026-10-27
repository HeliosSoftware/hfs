import type { Locator } from "@playwright/test";

export interface ButtonGeometry {
  height: string;
  paddingLeft: string;
  paddingRight: string;
  fontSize: string;
  radius: string;
}

export const CANONICAL_BUTTON_GEOMETRY: ButtonGeometry = {
  height: "30px",
  paddingLeft: "12px",
  paddingRight: "12px",
  fontSize: "12px",
  radius: "9px",
};

export async function readButtonGeometry(button: Locator): Promise<ButtonGeometry> {
  const geometries = await readButtonGeometries(button);
  if (geometries.length !== 1) {
    throw new Error(
      `readButtonGeometry expected exactly one matched element, got ${geometries.length}: ${button}`,
    );
  }
  return geometries[0];
}

export async function readButtonGeometries(buttons: Locator): Promise<ButtonGeometry[]> {
  return buttons.evaluateAll((elements) =>
    elements.map((element) => {
      const style = getComputedStyle(element);
      return {
        height: style.height,
        paddingLeft: style.paddingLeft,
        paddingRight: style.paddingRight,
        fontSize: style.fontSize,
        radius: style.borderRadius,
      };
    }),
  );
}
