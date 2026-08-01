import { TaskbarWidget } from "../shared/widget";
import { conductorWidget } from "./conductor";
import { systemWidget } from "./system";

const ALL: TaskbarWidget[] = [systemWidget, conductorWidget];

export function widgetsFor(enabledIds: string[]): TaskbarWidget[] {
  return enabledIds
    .map((id) => ALL.find((w) => w.id === id))
    .filter((w): w is TaskbarWidget => !!w);
}

export function widgetById(id: string): TaskbarWidget | undefined {
  return ALL.find((w) => w.id === id);
}
