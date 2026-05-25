import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface AccountRecord {
  account_id: string;
  provider: "gmail" | "graph";
  email: string;
}

export interface AccountSummary {
  account_id: string;
  email: string;
  provider: string;
  message_count: number;
  phase: "initial" | "incremental" | null;
  last_sync_at: string | null;
}

export interface ConnectAccountResult {
  account_id: string;
  email: string;
}

export interface SyncProgress {
  account_id: string;
  stage: "listing" | "fetching" | "incremental" | "done";
  messages_seen: number;
  messages_persisted: number;
  elapsed_ms: number;
}

export const tauriApi = {
  gmailConnectAccount: (): Promise<ConnectAccountResult> =>
    invoke<ConnectAccountResult>("gmail_connect_account"),
  gmailListAccounts: (): Promise<AccountRecord[]> =>
    invoke<AccountRecord[]>("gmail_list_accounts"),
  gmailAccountSummaries: (): Promise<AccountSummary[]> =>
    invoke<AccountSummary[]>("gmail_account_summaries"),
  gmailSync: (accountId: string): Promise<void> =>
    invoke<void>("gmail_sync", { accountId }),
  onSyncProgress: (handler: (p: SyncProgress) => void): Promise<UnlistenFn> =>
    listen<SyncProgress>("sync:progress", (event) => handler(event.payload)),
};
