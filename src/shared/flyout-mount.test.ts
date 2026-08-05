import { describe, expect, it, vi } from "vitest";
import { createFlyoutController } from "./flyout-mount";

function harness() {
  const cleanups: ReturnType<typeof vi.fn>[] = [];
  const mount = vi.fn((_id: string) => {
    const stop = vi.fn();
    cleanups.push(stop);
    return stop;
  });
  return { mount, cleanups, ctl: createFlyoutController(mount) };
}

describe("createFlyoutController", () => {
  it("mounts on first show and skips a repeat show of the same widget", () => {
    const { mount, ctl } = harness();
    ctl.show("cpu");
    ctl.show("cpu");
    expect(mount).toHaveBeenCalledTimes(1);
    expect(ctl.mounted()).toBe("cpu");
  });

  // The regression this file exists for: without hide() unmounting, cpu/ram/gpu
  // keep polling get_top_processes every 2s for the rest of the session.
  it("unmounts on hide", () => {
    const { cleanups, ctl } = harness();
    ctl.show("cpu");
    ctl.hide();
    expect(cleanups[0]).toHaveBeenCalledTimes(1);
    expect(ctl.mounted()).toBeNull();
  });

  it("remounts the same widget after a hide", () => {
    const { mount, ctl } = harness();
    ctl.show("cpu");
    ctl.hide();
    ctl.show("cpu");
    expect(mount).toHaveBeenCalledTimes(2);
  });

  it("unmounts the previous widget when a different one is shown", () => {
    const { cleanups, ctl } = harness();
    ctl.show("cpu");
    ctl.show("ram");
    expect(cleanups[0]).toHaveBeenCalledTimes(1);
    expect(cleanups[1]).not.toHaveBeenCalled();
  });

  it("never double-runs a cleanup across hide then hide", () => {
    const { cleanups, ctl } = harness();
    ctl.show("cpu");
    ctl.hide();
    ctl.hide();
    expect(cleanups[0]).toHaveBeenCalledTimes(1);
  });

  it("ignores a null id without disturbing what is mounted", () => {
    const { mount, cleanups, ctl } = harness();
    ctl.show("cpu");
    ctl.show(null);
    expect(mount).toHaveBeenCalledTimes(1);
    expect(cleanups[0]).not.toHaveBeenCalled();
    expect(ctl.mounted()).toBe("cpu");
  });
});
