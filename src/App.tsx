import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  tauriApi,
  type AccountSummary,
  type SyncProgress,
} from "./lib/tauri";

export function App(): JSX.Element {
  return (
    <main className="flex min-h-screen flex-col items-center bg-zinc-950 px-6 py-12 text-zinc-100">
      <header className="text-center">
        <h1 className="text-4xl font-semibold tracking-tight">Chhanni</h1>
        <p className="mt-2 text-sm text-zinc-400">
          On-device inbox cleanup. Nothing leaves your machine.
        </p>
      </header>
      <ConnectPanel />
    </main>
  );
}

function ConnectPanel(): JSX.Element {
  const queryClient = useQueryClient();
  const accountsQuery = useQuery({
    queryKey: ["accountSummaries"],
    queryFn: tauriApi.gmailAccountSummaries,
  });

  const connect = useMutation({
    mutationFn: tauriApi.gmailConnectAccount,
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
        <button
          type="button"
          onClick={() => connect.mutate()}
          disabled={connect.isPending}
          className="shrink-0 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-emerald-500 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {connect.isPending ? "Waiting…" : "Connect Gmail"}
        </button>
      </div>

      {connect.isError ? (
        <p className="mt-3 text-xs text-red-400">
          {connect.error instanceof Error ? connect.error.message : "Connection failed"}
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
    mutationFn: () => tauriApi.gmailSync(account.account_id),
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
        <button
          type="button"
          onClick={() => sync.mutate()}
          disabled={sync.isPending}
          className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 transition hover:bg-zinc-700 disabled:opacity-50"
        >
          {sync.isPending ? "Syncing…" : "Sync"}
        </button>
      </div>

      {progress && sync.isPending ? (
        <div className="mt-3 text-xs text-zinc-400">
          {progress.stage} · seen {progress.messages_seen.toLocaleString()} ·
          persisted {progress.messages_persisted.toLocaleString()} ·{" "}
          {(progress.elapsed_ms / 1000).toFixed(1)}s
        </div>
      ) : null}

      {sync.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {sync.error instanceof Error ? sync.error.message : "Sync failed"}
        </p>
      ) : null}
    </li>
  );
}
