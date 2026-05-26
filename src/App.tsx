import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  errorMessage,
  tauriApi,
  type AccountSummary,
  type SyncProgress,
} from "./lib/tauri";
import { History } from "./History";
import { Review } from "./Review";
import { Settings } from "./Settings";

type View = "accounts" | "review" | "history" | "settings";

export function App(): JSX.Element {
  const [view, setView] = useState<View>("accounts");
  const accountsQuery = useQuery({
    queryKey: ["accountSummaries"],
    queryFn: tauriApi.gmailAccountSummaries,
  });

  return (
    <main className="flex min-h-screen flex-col items-center bg-zinc-950 px-6 py-12 text-zinc-100">
      <header className="text-center">
        <h1 className="text-4xl font-semibold tracking-tight">Chhanni</h1>
        <p className="mt-2 text-sm text-zinc-400">
          On-device inbox cleanup. Nothing leaves your machine.
        </p>
      </header>
      <nav className="mt-6 flex gap-2 text-xs">
        {(["accounts", "review", "history", "settings"] as const).map((v) => (
          <button
            key={v}
            type="button"
            onClick={() => setView(v)}
            className={`rounded px-3 py-1.5 capitalize ${
              view === v
                ? "bg-emerald-700 text-white"
                : "bg-zinc-800 text-zinc-300 hover:bg-zinc-700"
            }`}
          >
            {v}
          </button>
        ))}
      </nav>
      {view === "accounts" ? (
        <ConnectPanel accountsQuery={accountsQuery} onGoToSettings={() => setView("settings")} />
      ) : view === "review" ? (
        <Review accounts={accountsQuery.data ?? []} />
      ) : view === "history" ? (
        <History accounts={accountsQuery.data ?? []} />
      ) : (
        <Settings />
      )}
    </main>
  );
}

interface ConnectPanelProps {
  accountsQuery: ReturnType<typeof useQuery<AccountSummary[], Error>>;
  onGoToSettings: () => void;
}

function ConnectPanel({ accountsQuery, onGoToSettings }: ConnectPanelProps): JSX.Element {
  const queryClient = useQueryClient();
  const oauthStatus = useQuery({
    queryKey: ["oauthStatus"],
    queryFn: tauriApi.oauthStatus,
  });

  const connectGmail = useMutation({
    mutationFn: tauriApi.gmailConnectAccount,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accountSummaries"] });
    },
  });
  const connectOutlook = useMutation({
    mutationFn: tauriApi.graphConnectAccount,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accountSummaries"] });
    },
  });

  return (
    <section className="mt-8 w-full max-w-2xl rounded-lg border border-zinc-800 bg-zinc-900/60 p-6 shadow-xl">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold">Accounts</h2>
          <p className="mt-1 text-xs text-zinc-400">
            Authorize Chhanni to read your Gmail. The connection runs locally;
            only your browser sees the consent screen.
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            onClick={() => connectGmail.mutate()}
            disabled={connectGmail.isPending || !(oauthStatus.data?.gmail_configured ?? false)}
            title={
              oauthStatus.data?.gmail_configured
                ? "Begin Gmail OAuth"
                : "Add a Gmail client ID in Settings first"
            }
            className="rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-emerald-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {connectGmail.isPending ? "Waiting…" : "Connect Gmail"}
          </button>
          <button
            type="button"
            onClick={() => connectOutlook.mutate()}
            disabled={connectOutlook.isPending || !(oauthStatus.data?.graph_configured ?? false)}
            title={
              oauthStatus.data?.graph_configured
                ? "Begin Microsoft OAuth"
                : "Add an Outlook client ID in Settings first"
            }
            className="rounded-md bg-sky-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {connectOutlook.isPending ? "Waiting…" : "Connect Outlook"}
          </button>
        </div>
      </div>

      {oauthStatus.data &&
      !oauthStatus.data.gmail_configured &&
      !oauthStatus.data.graph_configured ? (
        <div className="mt-3 rounded border border-amber-700/40 bg-amber-950/40 p-3 text-xs text-amber-200">
          <p>
            No OAuth credentials configured yet. Open{" "}
            <button
              type="button"
              onClick={onGoToSettings}
              className="underline hover:text-amber-100"
            >
              Settings
            </button>{" "}
            to add a Gmail or Outlook client ID.
          </p>
        </div>
      ) : null}

      {connectGmail.isError ? (
        <p className="mt-3 text-xs text-red-400">
          {errorMessage(connectGmail.error, "Gmail connection failed")}
        </p>
      ) : null}
      {connectOutlook.isError ? (
        <p className="mt-3 text-xs text-red-400">
          {errorMessage(connectOutlook.error, "Outlook connection failed")}
        </p>
      ) : null}

      <div className="mt-6 border-t border-zinc-800 pt-4">
        <AccountList
          data={accountsQuery.data ?? []}
          isLoading={accountsQuery.isLoading}
        />
      </div>
    </section>
  );
}

interface AccountListProps {
  data: AccountSummary[];
  isLoading: boolean;
}

function AccountList({ data, isLoading }: AccountListProps): JSX.Element {
  if (isLoading) {
    return <p className="text-xs text-zinc-500">Loading…</p>;
  }
  if (data.length === 0) {
    return <p className="text-xs text-zinc-500">No accounts connected yet.</p>;
  }
  return (
    <ul className="space-y-3">
      {data.map((account) => (
        <AccountCard key={account.account_id} account={account} />
      ))}
    </ul>
  );
}

interface AccountCardProps {
  account: AccountSummary;
}

function AccountCard({ account }: AccountCardProps): JSX.Element {
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void tauriApi
      .onSyncProgress((p) => {
        if (p.account_id === account.account_id) {
          setProgress(p);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
  }, [account.account_id]);

  const sync = useMutation({
    mutationFn: () =>
      account.provider === "graph"
        ? tauriApi.graphSync(account.account_id)
        : tauriApi.gmailSync(account.account_id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accountSummaries"] });
    },
  });

  const remove = useMutation({
    mutationFn: () => tauriApi.deleteAccount(account.account_id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accountSummaries"] });
    },
  });

  return (
    <li className="rounded border border-zinc-800 bg-zinc-950 p-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-medium">{account.email}</div>
          <div className="mt-0.5 text-xs uppercase tracking-wider text-zinc-500">
            {account.provider} · {account.message_count.toLocaleString()} messages
            {account.phase ? ` · ${account.phase}` : ""}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            onClick={() => sync.mutate()}
            disabled={sync.isPending || remove.isPending}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 transition hover:bg-zinc-700 disabled:opacity-50"
          >
            {sync.isPending ? "Syncing…" : "Sync"}
          </button>
          {confirmingDelete ? (
            <>
              <button
                type="button"
                onClick={() => {
                  setConfirmingDelete(false);
                  remove.mutate();
                }}
                disabled={remove.isPending}
                className="rounded-md bg-red-700 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-red-600 disabled:opacity-50"
              >
                {remove.isPending ? "Deleting…" : "Confirm delete"}
              </button>
              <button
                type="button"
                onClick={() => setConfirmingDelete(false)}
                disabled={remove.isPending}
                className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-300 transition hover:bg-zinc-700 disabled:opacity-50"
              >
                Cancel
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => setConfirmingDelete(true)}
              disabled={remove.isPending || sync.isPending}
              title="Disconnect account and delete all local data"
              className="rounded-md border border-red-900/60 bg-transparent px-3 py-1.5 text-xs font-medium text-red-300 transition hover:bg-red-950/40 disabled:opacity-50"
            >
              Delete
            </button>
          )}
        </div>
      </div>

      {confirmingDelete ? (
        <p className="mt-2 text-xs text-red-300">
          This removes the OAuth token and every locally stored message,
          embedding, and cluster for {account.email}. Irreversible.
        </p>
      ) : null}

      {progress && sync.isPending ? (
        <div className="mt-3 text-xs text-zinc-400">
          {progress.stage} · seen {progress.messages_seen.toLocaleString()} ·
          persisted {progress.messages_persisted.toLocaleString()} ·{" "}
          {(progress.elapsed_ms / 1000).toFixed(1)}s
        </div>
      ) : null}

      {sync.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(sync.error, "Sync failed")}
        </p>
      ) : null}

      {remove.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(remove.error, "Delete failed")}
        </p>
      ) : null}

      <ClusterPanel accountId={account.account_id} />
    </li>
  );
}

interface ClusterPanelProps {
  accountId: string;
}

function ClusterPanel({ accountId }: ClusterPanelProps): JSX.Element {
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: ["embeddingStatus", accountId],
    queryFn: () => tauriApi.embeddingStatus(accountId),
  });
  const clusters = useQuery({
    queryKey: ["clusters", accountId],
    queryFn: () => tauriApi.listClusters(accountId),
  });

  const [sidecarPort, setSidecarPort] = useState(() => {
    const stored = window.localStorage.getItem("chhanni:sidecarPort");
    return stored ? Number(stored) : 8080;
  });
  const [classifierPort, setClassifierPort] = useState(() => {
    const stored = window.localStorage.getItem("chhanni:classifierPort");
    return stored ? Number(stored) : 8081;
  });

  useEffect(() => {
    window.localStorage.setItem("chhanni:sidecarPort", String(sidecarPort));
  }, [sidecarPort]);
  useEffect(() => {
    window.localStorage.setItem("chhanni:classifierPort", String(classifierPort));
  }, [classifierPort]);

  const categories = useQuery({
    queryKey: ["categories", accountId],
    queryFn: () => tauriApi.classificationSummary(accountId),
  });

  const embed = useMutation({
    mutationFn: () => tauriApi.embedRun(accountId, sidecarPort),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["embeddingStatus", accountId] });
    },
  });

  const cluster = useMutation({
    mutationFn: () => tauriApi.clusterRun(accountId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["clusters", accountId] });
      void queryClient.invalidateQueries({ queryKey: ["embeddingStatus", accountId] });
    },
  });

  const classify = useMutation({
    mutationFn: () => tauriApi.classifyRun(accountId, classifierPort),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["categories", accountId] });
    },
  });

  return (
    <div className="mt-3 border-t border-zinc-800 pt-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs text-zinc-500">
          {status.data
            ? `${status.data.embedded.toLocaleString()} / ${status.data.total.toLocaleString()} embedded · ${status.data.clusters} clusters`
            : "—"}
        </div>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1 text-[10px] text-zinc-500">
            embed
            <input
              type="number"
              value={sidecarPort}
              onChange={(e) => setSidecarPort(Number(e.target.value))}
              className="w-16 rounded border border-zinc-800 bg-zinc-950 px-1 py-1 text-xs text-zinc-200"
              title="embedding llama-server port"
            />
          </label>
          <label className="flex items-center gap-1 text-[10px] text-zinc-500">
            classify
            <input
              type="number"
              value={classifierPort}
              onChange={(e) => setClassifierPort(Number(e.target.value))}
              className="w-16 rounded border border-zinc-800 bg-zinc-950 px-1 py-1 text-xs text-zinc-200"
              title="classifier llama-server port"
            />
          </label>
          <button
            type="button"
            onClick={() => embed.mutate()}
            disabled={embed.isPending}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 transition hover:bg-zinc-700 disabled:opacity-50"
          >
            {embed.isPending ? "Embedding…" : "Embed"}
          </button>
          <button
            type="button"
            onClick={() => cluster.mutate()}
            disabled={cluster.isPending}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 transition hover:bg-zinc-700 disabled:opacity-50"
          >
            {cluster.isPending ? "Clustering…" : "Cluster"}
          </button>
          <button
            type="button"
            onClick={() => classify.mutate()}
            disabled={classify.isPending}
            className="rounded-md bg-emerald-700 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-emerald-600 disabled:opacity-50"
          >
            {classify.isPending ? "Classifying…" : "Classify"}
          </button>
        </div>
      </div>

      {categories.data && categories.data.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1 text-[10px] text-zinc-400">
          {categories.data.map((b) => (
            <span
              key={b.category}
              className="rounded bg-zinc-800/60 px-1.5 py-0.5"
              title={`${b.count} messages`}
            >
              {b.category}: {b.count.toLocaleString()}
            </span>
          ))}
        </div>
      ) : null}

      {embed.isSuccess ? (
        <p className="mt-2 text-xs text-emerald-400">
          Embedded {embed.data.toLocaleString()} message
          {embed.data === 1 ? "" : "s"}.
        </p>
      ) : null}
      {embed.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(embed.error, "Embedding failed")}
        </p>
      ) : null}

      {cluster.isSuccess ? (
        <p className="mt-2 text-xs text-emerald-400">
          {cluster.data === 0
            ? "No clusters created — embed messages first."
            : `Built ${cluster.data.toLocaleString()} cluster${cluster.data === 1 ? "" : "s"}.`}
        </p>
      ) : null}
      {cluster.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(cluster.error, "Clustering failed")}
        </p>
      ) : null}

      {classify.isSuccess ? (
        <p className="mt-2 text-xs text-emerald-400">
          {classify.data === 0
            ? "Nothing new to classify — either no clusters yet or they're already up to date."
            : `Classified ${classify.data.toLocaleString()} cluster${classify.data === 1 ? "" : "s"}.`}
        </p>
      ) : null}
      {classify.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(classify.error, "Classification failed")}
        </p>
      ) : null}

      {clusters.data && clusters.data.length > 0 ? (
        <ul className="mt-3 max-h-48 space-y-1 overflow-y-auto text-xs text-zinc-300">
          {clusters.data.slice(0, 10).map((c) => (
            <li key={c.cluster_key} className="flex items-center justify-between gap-3">
              <span className="truncate">
                {c.sample_sender ?? c.cluster_key} · {c.sample_subject ?? "(no subject)"}
              </span>
              <span className="shrink-0 text-zinc-500">{c.member_count}</span>
            </li>
          ))}
          {clusters.data.length > 10 ? (
            <li className="text-zinc-500">…and {clusters.data.length - 10} more</li>
          ) : null}
        </ul>
      ) : null}

    </div>
  );
}
