import { isDividerId } from "../shared/divider";
import { TaskbarWidget } from "../shared/widget";
import { conductorWidget } from "./conductor";
import { cpuWidget } from "./cpu";
import { dividerWidget } from "./divider";
import { diskWidget } from "./disk";
import { gpuWidget } from "./gpu";
import { pomodoroWidget } from "./pomodoro";
import { ramWidget } from "./ram";

const ALL: TaskbarWidget[] = [
  cpuWidget,
  ramWidget,
  gpuWidget,
  diskWidget,
  conductorWidget,
  pomodoroWidget,
];

// Divider ids are minted per-instance (shared/divider.ts) rather than declared here,
// so they're synthesized on the fly instead of looked up in ALL.
export function widgetsFor(enabledIds: string[]): TaskbarWidget[] {
  return enabledIds
    .map((id) => (isDividerId(id) ? dividerWidget(id) : ALL.find((w) => w.id === id)))
    .filter((w): w is TaskbarWidget => !!w);
}

export function widgetById(id: string): TaskbarWidget | undefined {
  if (isDividerId(id)) return dividerWidget(id);
  return ALL.find((w) => w.id === id);
}

export function allWidgets(): TaskbarWidget[] {
  return ALL;
}

export function allWidgetIds(): string[] {
  return ALL.map((w) => w.id);
}
