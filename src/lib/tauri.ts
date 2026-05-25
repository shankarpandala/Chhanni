import { invoke } from "@tauri-apps/api/core";

export interface AccountRecord {
  account_id: string;
  provider: "gmail" | "graph";
  email: string;
}

export interface ConnectAccountResult {
  account_id: string;
  email: string;
}

export const tauriApi = {
  gmailConnectAccount: (): Promise<ConnectAccountResult> =>
    invoke<ConnectAccountResult>("gmail_connect_account"),
  gmailListAccounts: (): Promise<AccountRecord[]> =>
    invoke<AccountRecord[]>("gmail_list_accounts"),
};
