import { TaskbarWidget } from "../shared/widget";
import { conductorWidget } from "./conductor";
import { cpuWidget } from "./cpu";
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

export function widgetsFor(enabledIds: string[]): TaskbarWidget[] {
  return enabledIds
    .map((id) => ALL.find((w) => w.id === id))
    .filter((w): w is TaskbarWidget => !!w);
}

export function widgetById(id: string): TaskbarWidget | undefined {
  return ALL.find((w) => w.id === id);
}

export function allWidgets(): TaskbarWidget[] {
  return ALL;
}

export function allWidgetIds(): string[] {
  return ALL.map((w) => w.id);
}
