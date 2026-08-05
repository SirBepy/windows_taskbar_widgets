import { defineSchema } from "../../../vendor/tauri_kit/frontend/settings/schema";
import { widgetListField } from "./widget-list-field";
import { autostartField } from "./autostart-field";

export const settingsSchema = defineSchema({
  sections: [
    {
      title: "Widgets",
      fields: [widgetListField()],
    },
    {
      title: "Host",
      fields: [
        {
          key: "follow_taskbar",
          kind: "toggle",
          label: "Only show when the taskbar is visible",
        },
        {
          key: "hide_from_capture",
          kind: "toggle",
          label: "Hide from screen sharing and recording",
          tooltip:
            "The strip becomes invisible in screen shares and recordings - and in your own screenshots too.",
        },
        {
          key: "left_margin",
          kind: "number",
          label: "Left margin (px)",
          min: 0,
        },
        {
          key: "stats_poll_seconds",
          kind: "integer",
          label: "Stats poll interval (s)",
          min: 1,
        },
        {
          key: "opacity",
          kind: "range",
          label: "Widget opacity (%)",
          min: 0,
          max: 100,
          step: 1,
        },
      ],
    },
  ],
});

/** Rendered on the kit's System page, above the danger zone. */
export const systemInline = [autostartField()];
