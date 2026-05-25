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

export interface ClusterSummary {
  cluster_key: string;
  sample_sender: string | null;
  sample_subject: string | null;
  member_count: number;
  last_internal_date: number;
}

export interface EmbeddingStatus {
  total: number;
  embedded: number;
  clusters: number;
}

export interface EmbedProgress {
  account_id: string;
  processed: number;
  remaining: number;
  elapsed_ms: number;
}

export interface BootstrapProgress {
  kind: "binary" | "model";
  bytes_downloaded: number;
  bytes_total: number | null;
}

export interface ClassifyProgress {
  account_id: string;
  processed: number;
  remaining: number;
  skipped_unchanged: number;
  elapsed_ms: number;
}

export interface CategoryBucket {
  category: string;
  count: number;
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

  embedBootstrap: (): Promise<string> => invoke<string>("embed_bootstrap"),
  classifierBootstrap: (): Promise<string> => invoke<string>("classifier_bootstrap"),
  embedRun: (accountId: string, sidecarPort: number): Promise<number> =>
    invoke<number>("embed_run", { accountId, sidecarPort }),
  clusterRun: (accountId: string): Promise<number> =>
    invoke<number>("cluster_run", { accountId }),
  classifyRun: (accountId: string, classifierPort: number): Promise<number> =>
    invoke<number>("classify_run", { accountId, classifierPort }),
  listClusters: (accountId: string): Promise<ClusterSummary[]> =>
    invoke<ClusterSummary[]>("list_clusters", { accountId }),
  embeddingStatus: (accountId: string): Promise<EmbeddingStatus> =>
    invoke<EmbeddingStatus>("embedding_status", { accountId }),
  classificationSummary: (accountId: string): Promise<CategoryBucket[]> =>
    invoke<CategoryBucket[]>("classification_summary", { accountId }),
  classificationCount: (accountId: string): Promise<number> =>
    invoke<number>("classification_count", { accountId }),

  onEmbedProgress: (
    handler: (p: EmbedProgress) => void,
  ): Promise<UnlistenFn> =>
    listen<EmbedProgress>("embed:progress", (event) => handler(event.payload)),
  onClassifyProgress: (
    handler: (p: ClassifyProgress) => void,
  ): Promise<UnlistenFn> =>
    listen<ClassifyProgress>("classify:progress", (event) =>
      handler(event.payload),
    ),
  onBootstrapProgress: (
    handler: (p: BootstrapProgress) => void,
  ): Promise<UnlistenFn> =>
    listen<BootstrapProgress>("embed:bootstrap", (event) =>
      handler(event.payload),
    ),
};
