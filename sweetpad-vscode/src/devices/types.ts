import type { DeviceCtlDevice, DeviceCtlDeviceType } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";
import type { IDestination } from "../destination/types";
import { resolveDeviceType } from "./merge";
import { supportsDevicectl } from "./utils";

type DeviceState = "connected" | "disconnected" | "unavailable";

/**
 * Paired raw records for a single physical device. At least one of "devicectl" or
 * "xcdevice" is always set — enforced by DeviceDestinationBase's constructor.
 *
 * The two fields come from different Apple CLIs:
 *
 * - "xcrun devicectl" — Apple's modern device-management CLI (iOS 17+ / CoreDevice).
 *   Rich JSON, supports install/launch/process-info. Unreliable for iOS <= 16: USB
 *   devices come back with empty "hardwareProperties"; Wi-Fi devices are omitted.
 *
 * - "xcrun xcdevice" — older inventory-only tool. Reports every paired device across
 *   all supported platforms, including iOS <= 16 ones devicectl drops. Cannot install
 *   or launch apps; deploy via "ios-deploy" instead.
 */
export type DeviceRaw = {
  devicectl?: DeviceCtlDevice;
  xcdevice?: XcdeviceDevice;
};

/**
 * Shared data + fallback logic for physical Apple devices, regardless of platform.
 *
 * Why this exists: "xcrun devicectl" (used for iOS 17+) and "xcrun xcdevice" (used for
 * iOS <= 16) return overlapping but differently shaped records. This base class holds
 * both sources ("raw.devicectl" / "raw.xcdevice") and exposes unified getters so
 * subclasses only need to supply platform-specific constants (type/typeLabel/platform/
 * idPrefix/minDevicectlMajor) and platform-specific icon logic.
 *
 * At least one of "devicectl" or "xcdevice" must be set — the constructor throws otherwise.
 * The raw records remain accessible via "this.raw.*" for the rare case where a consumer
 * needs source-specific fields; prefer the unified getters.
 */
export abstract class DeviceDestinationBase {
  public readonly raw: DeviceRaw;

  /** Prefix used to build the destination id (e.g. "iosdevice"). */
  protected abstract readonly idPrefix: string;

  /** Minimum major OS version that supports devicectl on this platform. */
  protected abstract readonly minDevicectlMajor: number;

  /** Human-readable platform label shown in quick-pick details. */
  abstract readonly typeLabel: string;

  /** Platform-specific icon logic. */
  abstract get icon(): string;

  constructor(raw: DeviceRaw) {
    if (!raw.devicectl && !raw.xcdevice) {
      throw new Error("Device requires at least one of devicectl or xcdevice records");
    }
    this.raw = raw;
  }

  get id(): string {
    return `${this.idPrefix}-${this.udid}`;
  }

  get label(): string {
    const v = this.osVersion;
    return v === "Unknown" ? this.name : `${this.name} (${v})`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
  }

  get isConnected(): boolean {
    return this.state === "connected";
  }

  /** True when "xcrun devicectl" reported this device. Gate devicectl-only operations on this. */
  get hasDevicectl(): boolean {
    return this.raw.devicectl !== undefined;
  }

  /** True when "xcrun xcdevice" reported this device. Mostly useful for tests/introspection. */
  get hasXcdevice(): boolean {
    return this.raw.xcdevice !== undefined;
  }

  /**
   * The device's UDID in its long-standing hex form (e.g. "00008110-001234567890001E").
   *
   * Sometimes called "legacy" because Apple introduced a second identifier format
   * alongside devicectl in Xcode 15 — a URN like "urn:x-ios-devicectl:device-CS1234567-..."
   * (see "devicectlId"). The hex UDID predates that and is what most tooling still speaks:
   * "xcodebuild -destination id=...", "ios-deploy --id", Instruments. Not deprecated —
   * it remains the portable identifier for everything outside devicectl itself.
   */
  get udid(): string {
    const dc = this.raw.devicectl;
    const xc = this.raw.xcdevice;
    return dc?.hardwareProperties?.udid ?? xc?.identifier ?? dc?.identifier ?? "unknown";
  }

  /**
   * devicectl-internal identifier (URN form, e.g. "urn:x-ios-devicectl:device-CS1234567-...").
   * Null when the device is only known to xcdevice — callers that need this must gate on
   * "hasDevicectl" / "supportsDevicectl".
   */
  get devicectlId(): string | null {
    return this.raw.devicectl?.identifier ?? null;
  }

  /**
   * Human-readable device name shown to the user — the label in the destinations tree,
   * the "Running ... on <name>" progress status, and the quick-pick details string.
   * Prefers the user-customized name (e.g. "John's iPhone") over marketing names.
   */
  get name(): string {
    const dc = this.raw.devicectl;
    const xc = this.raw.xcdevice;
    const dcName = dc?.deviceProperties?.name;
    const marketing = dc?.hardwareProperties?.marketingName;

    // devicectl sometimes returns the marketing name as the device name for iOS <17 —
    // in that case prefer xcdevice's customized name.
    // e.g. dcName="John's iPhone", marketing="iPhone 15 Pro" → "John's iPhone"
    if (dcName && dcName !== marketing) {
      return dcName;
    }

    // e.g. dcName="iPhone 13" (==marketing), xc.name="Nixuge iPhone" → "Nixuge iPhone"
    if (xc?.name) {
      return xc.name;
    }

    // e.g. dcName="iPhone 15 Pro", no xcdevice entry → "iPhone 15 Pro"
    if (dcName) {
      return dcName;
    }

    // e.g. dcName undefined but marketing="iPad Pro 12.9-inch" → "iPad Pro 12.9-inch"
    if (marketing) {
      return marketing;
    }

    // Last-resort fallback: raw model code like "iPhone14,2".
    const modelCode = dc?.hardwareProperties?.productType ?? xc?.modelCode;
    return modelCode ?? "Unknown Device";
  }

  /**
   * OS version string shown in the destinations tree description and the quick-pick,
   * and used by "supportsDevicectl" to decide devicectl vs ios-deploy routing.
   *
   * Always returned in plain form ("17.0", "16.4.1") regardless of source:
   * - devicectl → plain version already ("17.0")
   * - xcdevice  → "16.4.1 (20E252)" in the wild; the build suffix is stripped so
   *   the label doesn't render nested parens ("My iPhone (16.4.1 (20E252))") and
   *   so "supportsDevicectl"'s leading-digit parse is not fragile.
   *
   * Returns "Unknown" when neither source reports a version.
   */
  get osVersion(): string {
    const dcVersion = this.raw.devicectl?.deviceProperties?.osVersionNumber;
    if (dcVersion) {
      return dcVersion;
    }
    const xcVersion = this.raw.xcdevice?.operatingSystemVersion;
    if (!xcVersion || xcVersion.length === 0) {
      return "Unknown";
    }
    // "16.4.1 (20E252)" → "16.4.1"; leave plain strings untouched.
    const spaceIdx = xcVersion.indexOf(" ");
    return spaceIdx === -1 ? xcVersion : xcVersion.slice(0, spaceIdx);
  }

  /**
   * iPhone / iPad / appleWatch / appleTV / appleVision / realityDevice.
   *
   * devicectl reports this directly. For xcdevice-only devices, the subclass picked
   * by the merge layer narrows this — e.g. iOSDeviceDestination only ever wraps
   * iphoneos-platform xcdevice entries, so inference boils down to iPhone vs iPad.
   */
  get deviceType(): DeviceCtlDeviceType {
    // buildDeviceDestination already drops records resolveDeviceType can't classify,
    // so any instance that reaches this point has a resolvable type. Throw on the
    // impossible path rather than silently defaulting to iPhone.
    const t = resolveDeviceType(this.raw);
    if (!t) {
      throw new Error("DeviceDestinationBase.deviceType: unclassifiable raw record");
    }
    return t;
  }

  /**
   * Connection/availability state used by "isConnected" and the icon variant.
   *
   * - "connected"    → device reachable; deploys will succeed.
   * - "disconnected" → devicectl tunnelState=disconnected (cable pulled, etc.).
   * - "unavailable"  → devicectl tunnelState=unavailable, OR xcdevice says
   *                    available=false / returned an error (not paired, locked,
   *                    developer mode off).
   */
  get state(): DeviceState {
    // e.g. devicectl.connectionProperties.tunnelState="connected" → "connected"
    const dc = this.raw.devicectl;
    if (dc) {
      return dc.connectionProperties.tunnelState;
    }
    const xc = this.raw.xcdevice;
    if (!xc) {
      // unreachable: constructor guarantees at least one source
      return "unavailable";
    }
    // e.g. xcdevice { available: false, error: { code: -9 } } → "unavailable"
    if (xc.error || xc.available === false) {
      return "unavailable";
    }
    // e.g. xcdevice { available: true } → "connected"
    return "connected";
  }

  get supportsDevicectl(): boolean {
    if (!this.hasDevicectl) {
      return false;
    }
    const v = this.osVersion === "Unknown" ? undefined : this.osVersion;
    return supportsDevicectl(v, this.minDevicectlMajor);
  }

  /**
   * Timestamp of the device's most recent connection, from devicectl. Used by the
   * destination sort to surface recently-used devices ahead of long-stale paired
   * entries. Null for xcdevice-only devices (iOS <= 16) and when devicectl omits the
   * field. Invalid date strings also yield null so callers can treat "unknown" as oldest.
   */
  get lastConnectionDate(): Date | null {
    const raw = this.raw.devicectl?.connectionProperties?.lastConnectionDate;
    if (!raw) {
      return null;
    }
    const date = new Date(raw);
    return Number.isNaN(date.getTime()) ? null : date;
  }
}

export class iOSDeviceDestination extends DeviceDestinationBase implements IDestination {
  type = "iOSDevice" as const;
  typeLabel = "iOS Device";
  platform = "iphoneos" as const;
  protected readonly idPrefix = "iosdevice";
  protected readonly minDevicectlMajor = 17;

  get icon(): string {
    if (this.deviceType === "iPad") {
      return this.isConnected ? "sweetpad-device-ipad" : "sweetpad-device-ipad-x";
    }
    if (this.deviceType === "iPhone") {
      return this.isConnected ? "sweetpad-device-mobile" : "sweetpad-device-mobile-x";
    }
    return "sweetpad-device-mobile";
  }
}

export class watchOSDeviceDestination extends DeviceDestinationBase implements IDestination {
  type = "watchOSDevice" as const;
  typeLabel = "watchOS Device";
  platform = "watchos" as const;
  protected readonly idPrefix = "watchosdevice";
  protected readonly minDevicectlMajor = 10;

  get icon(): string {
    return this.isConnected ? "sweetpad-device-watch" : "sweetpad-device-watch-pause";
  }
}

export class tvOSDeviceDestination extends DeviceDestinationBase implements IDestination {
  type = "tvOSDevice" as const;
  typeLabel = "tvOS Device";
  platform = "appletvos" as const;
  protected readonly idPrefix = "tvosdevice";
  protected readonly minDevicectlMajor = 17;

  get icon(): string {
    return "sweetpad-device-tv-old";
  }
}

export class visionOSDeviceDestination extends DeviceDestinationBase implements IDestination {
  type = "visionOSDevice" as const;
  typeLabel = "visionOS Device";
  platform = "xros" as const;
  protected readonly idPrefix = "visionosdevice";
  protected readonly minDevicectlMajor = 1;

  get icon(): string {
    return "sweetpad-cardboards";
  }
}

export type DeviceDestination =
  | iOSDeviceDestination
  | watchOSDeviceDestination
  | tvOSDeviceDestination
  | visionOSDeviceDestination;
