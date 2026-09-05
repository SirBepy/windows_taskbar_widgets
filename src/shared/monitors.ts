/** The return shape of the `list_taskbar_monitors` command. Lives here rather than in a
 * view so a second consumer does not have to import a type from a lane renderer. */
export interface MonitorOption {
  device_name: string;
  is_primary: boolean;
  width: number;
  height: number;
}
