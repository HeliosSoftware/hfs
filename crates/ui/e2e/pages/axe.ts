// Renders axe violations the way a11y.spec.ts reports them, so a red run
// names the rule and the offending node instead of dumping raw result objects.
type Violation = {
  id: string;
  help: string;
  nodes: { target: unknown[]; failureSummary?: string }[];
};

export function axeSummary(violations: Violation[]): string {
  return violations
    .map(
      (v) =>
        `${v.id}: ${v.help}\n` +
        v.nodes.map((n) => `  ${n.target.join(" ")} — ${n.failureSummary ?? ""}`).join("\n"),
    )
    .join("\n");
}
