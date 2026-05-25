import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { tauriApi, type AccountSummary } from "./lib/tauri";

interface HistoryProps {
  accounts: AccountSummary[];
}

export function History({ accounts }: HistoryProps): JSX.Element {
  const [accountId, setAccountId] = useState<string | null>(
    accounts[0]?.account_id ?? null,
  );
  if (accountId === null) {
    return <p className="mt-8 text-sm text-zinc-500">Connect an account first.</p>;
  }
  return (
    <div className="mt-8 w-full max-w-4xl">
      {accounts.length > 1 ? (
        <div className="mb-3 flex gap-2 text-xs">
          {accounts.map((a) => (
            <button
              key={a.account_id}
              type="button"
              onClick={() => setAccountId(a.account_id)}
              className={`rounded px-2 py-1 ${
                a.account_id === accountId
                  ? "bg-emerald-700 text-white"
                  : "bg-zinc-800 text-zinc-300 hover:bg-zinc-700"
              }`}
            >
              {a.email}
            </button>
          ))}
        </div>
      ) : null}
      <HistoryBody accountId={accountId} />
    </div>
  );
}

function HistoryBody({ accountId }: { accountId: string }): JSX.Element {
  const queryClient = useQueryClient();
  const reversible = useQuery({
    queryKey: ["reversible", accountId],
    queryFn: () => tauriApi.listReversible(accountId, 100),
  });
  const counts = useQuery({
    queryKey: ["actionsLogCounts", accountId],
    queryFn: () => tauriApi.actionsLogCounts(accountId),
  });

  const undo = useMutation({
    mutationFn: (logId: number) => tauriApi.undoAction(logId, accountId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["reversible", accountId] });
      void queryClient.invalidateQueries({ queryKey: ["actionsLogCounts", accountId] });
    },
  });

  const exportTo = async (format: "json" | "csv") => {
    const result = await tauriApi.exportAudit(accountId, format);
    const blob = new Blob([result.content], {
      type: format === "json" ? "application/json" : "text/csv",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `chhanni-audit-${accountId}-${Date.now()}.${format}`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <section className="rounded-lg border border-zinc-800 bg-zinc-900/60 p-6">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">History</h2>
          <p className="mt-1 text-xs text-zinc-400">
            {counts.data?.success ?? 0} succeeded ·{" "}
            {counts.data?.failure ?? 0} failed ·{" "}
            {counts.data?.cancelled ?? 0} cancelled
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => void exportTo("csv")}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 hover:bg-zinc-700"
          >
            Export CSV
          </button>
          <button
            type="button"
            onClick={() => void exportTo("json")}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-100 hover:bg-zinc-700"
          >
            Export JSON
          </button>
        </div>
      </div>

      <h3 className="mt-4 text-sm font-medium">Reversible actions</h3>
      {reversible.isLoading ? (
        <p className="mt-2 text-xs text-zinc-500">Loading…</p>
      ) : null}
      {reversible.data && reversible.data.length === 0 ? (
        <p className="mt-2 text-xs text-zinc-500">No reversible actions yet.</p>
      ) : null}
      <ul className="mt-2 space-y-1 text-xs">
        {reversible.data?.map((entry) => (
          <li
            key={entry.id}
            className="flex items-center justify-between rounded border border-zinc-800 bg-zinc-950 px-3 py-2"
          >
            <div className="min-w-0">
              <div className="truncate font-medium text-zinc-200">
                {entry.action_type} · {entry.provider_msg_id}
              </div>
              <div className="truncate text-zinc-500">
                {entry.executed_at} · {entry.cluster_key}
              </div>
            </div>
            <button
              type="button"
              onClick={() => undo.mutate(entry.id)}
              disabled={undo.isPending}
              className="shrink-0 rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-100 hover:bg-zinc-700 disabled:opacity-50"
            >
              Undo
            </button>
          </li>
        ))}
      </ul>
      {undo.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {undo.error instanceof Error ? undo.error.message : "Undo failed"}
        </p>
      ) : null}
    </section>
  );
}
