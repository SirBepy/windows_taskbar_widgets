/** The return shape of the `list_taskbar_monitors` command. Lives here rather than in a
 * view so a second consumer does not have to import a type from a lane renderer. */
export interface MonitorOption {
  device_name: string;
  is_primary: boolean;
  width: number;
  height: number;
}

/** Windows' own numbering: the 2 in `\\.\DISPLAY2` is the "2" its Display settings
 * shows. Falls back to the caller's list order for a device name outside that shape.
 * `i` is the index in the unfiltered `list_taskbar_monitors` result, so a caller that
 * filters entries out must still pass the pre-filter index. */
export function monitorNumber(m: MonitorOption, i: number): string {
  return /DISPLAY(\d+)$/.exec(m.device_name)?.[1] ?? String(i + 1);
}
