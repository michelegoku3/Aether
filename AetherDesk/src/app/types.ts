export interface DeskUpdateInfo {
  update_available?: boolean;
  is_test?: boolean;
  installed_version?: string;
}

export interface DllUpdateInfo {
  update_available?: boolean;
  is_test?: boolean;
  installed_version?: string;
}

export interface HubcapUsageStats {
  usage: number;
  limit: number;
}

export interface HubcapUsage {
  usage: number;
  limit: number;
  hasKey: boolean;
}
