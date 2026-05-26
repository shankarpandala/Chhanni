import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  errorMessage,
  tauriApi,
  type AccountSummary,
  type ActionType,
  type ClusterSampleMessage,
  type ExecuteProgress,
  type ReviewClusterEntry,
} from "./lib/tauri";

interface ReviewProps {
  accounts: AccountSummary[];
}

export function Review({ accounts }: ReviewProps): JSX.Element {
  const [accountId, setAccountId] = useState<string | null>(
    accounts[0]?.account_id ?? null,
  );

  useEffect(() => {
    if (accountId === null && accounts[0]) {
      setAccountId(accounts[0].account_id);
    }
  }, [accountId, accounts]);

  if (accountId === null) {
    return (
      <div className="mt-8 text-sm text-zinc-500">
        Connect an account first.
      </div>
    );
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
      <ReviewBody accountId={accountId} />
    </div>
  );
}

function ReviewBody({ accountId }: { accountId: string }): JSX.Element {
  const queue = useQuery({
    queryKey: ["reviewQueue", accountId],
    queryFn: () => tauriApi.listReviewQueue(accountId),
  });

  const [filter, setFilter] = useState<"all" | "with-proposed" | "staged">(
    "with-proposed",
  );

  const entries = useMemo(() => {
    const all = queue.data?.entries ?? [];
    switch (filter) {
      case "with-proposed":
        return all.filter((e) => e.proposed.length > 0);
      case "staged":
        return all.filter((e) => e.staged_action_types.length > 0);
      default:
        return all;
    }
  }, [queue.data, filter]);

  return (
    <section className="rounded-lg border border-zinc-800 bg-zinc-900/60 p-6">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Review queue</h2>
          <p className="mt-1 text-xs text-zinc-400">
            Classifier suggested {queue.data?.entries?.length ?? 0} clusters
            · {queue.data?.staged_total ?? 0} staged so far
          </p>
        </div>
        <div className="flex gap-1 text-xs">
          {(["with-proposed", "staged", "all"] as const).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              className={`rounded px-2 py-1 ${
                filter === f
                  ? "bg-emerald-700 text-white"
                  : "bg-zinc-800 text-zinc-300 hover:bg-zinc-700"
              }`}
            >
              {f.replace("-", " ")}
            </button>
          ))}
        </div>
      </div>

      <ExecutorPanel
        accountId={accountId}
        stagedTotal={queue.data?.staged_total ?? 0}
      />

      {queue.isLoading ? (
        <p className="mt-4 text-xs text-zinc-500">Loading…</p>
      ) : null}

      {entries.length === 0 ? (
        <p className="mt-4 text-xs text-zinc-500">No clusters match.</p>
      ) : (
        <ul className="mt-4 space-y-2">
          {entries.map((entry) => (
            <ClusterRow
              key={entry.cluster_key}
              accountId={accountId}
              entry={entry}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function ExecutorPanel({
  accountId,
  stagedTotal,
}: {
  accountId: string;
  stagedTotal: number;
}): JSX.Element {
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<ExecuteProgress | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void tauriApi
      .onExecuteProgress((p) => {
        if (p.account_id === accountId) {
          setProgress(p);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
  }, [accountId]);

  const counts = useQuery({
    queryKey: ["actionsLogCounts", accountId],
    queryFn: () => tauriApi.actionsLogCounts(accountId),
  });

  const run = useMutation({
    mutationFn: () => tauriApi.runExecutor(accountId),
    onMutate: () => {
      setRunning(true);
      setProgress(null);
    },
    onSettled: () => {
      setRunning(false);
      void queryClient.invalidateQueries({ queryKey: ["reviewQueue", accountId] });
      void queryClient.invalidateQueries({ queryKey: ["actionsLogCounts", accountId] });
    },
  });

  const cancel = useMutation({
    mutationFn: () => tauriApi.cancelExecutor(accountId),
  });

  return (
    <div className="mt-4 rounded border border-zinc-800 bg-zinc-950 p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-medium">Execute staged actions</div>
          <div className="mt-0.5 text-xs text-zinc-400">
            {stagedTotal} action{stagedTotal === 1 ? "" : "s"} queued ·{" "}
            {counts.data?.success ?? 0} succeeded all-time ·{" "}
            {counts.data?.failure ?? 0} failed
          </div>
        </div>
        <div className="flex gap-2">
          {running ? (
            <button
              type="button"
              onClick={() => cancel.mutate()}
              disabled={cancel.isPending}
              className="rounded-md bg-red-700 px-3 py-1.5 text-xs font-medium text-white hover:bg-red-600 disabled:opacity-50"
            >
              Cancel
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => run.mutate()}
            disabled={running || stagedTotal === 0}
            className="rounded-md bg-emerald-700 px-4 py-1.5 text-xs font-medium text-white transition hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {running ? "Running…" : "Run cleanup"}
          </button>
        </div>
      </div>
      {progress ? (
        <div className="mt-2 text-xs text-zinc-400">
          batch {progress.batches_done} · {progress.messages_done.toLocaleString()} done ·{" "}
          {progress.failures.toLocaleString()} failed ·{" "}
          {(progress.elapsed_ms / 1000).toFixed(1)}s
        </div>
      ) : null}
      {run.isError ? (
        <p className="mt-2 text-xs text-red-400">
          {errorMessage(run.error, "Execution failed")}
        </p>
      ) : null}
    </div>
  );
}

function ClusterRow({
  accountId,
  entry,
}: {
  accountId: string;
  entry: ReviewClusterEntry;
}): JSX.Element {
  const queryClient = useQueryClient();
  const [expanded, setExpanded] = useState(false);

  const samples = useQuery({
    queryKey: ["clusterSamples", accountId, entry.cluster_key],
    queryFn: () => tauriApi.expandCluster(accountId, entry.cluster_key, 5),
    enabled: expanded,
  });

  const invalidateQueue = () =>
    queryClient.invalidateQueries({ queryKey: ["reviewQueue", accountId] });

  const stage = useMutation({
    mutationFn: ({
      action,
      reason,
    }: {
      action: ActionType;
      reason: string | null;
    }) => tauriApi.stageAction(accountId, entry.cluster_key, action, reason),
    onSuccess: () => {
      void invalidateQueue();
    },
  });

  const unstage = useMutation({
    mutationFn: (action: ActionType) =>
      tauriApi.unstageAction(accountId, entry.cluster_key, action),
    onSuccess: () => {
      void invalidateQueue();
    },
  });

  return (
    <li className="rounded border border-zinc-800 bg-zinc-950 p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">
            {entry.sample_sender ?? entry.cluster_key}
          </div>
          <div className="mt-0.5 truncate text-xs text-zinc-400">
            {entry.sample_subject ?? "(no subject)"}
          </div>
          <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-zinc-500">
            <span className="rounded bg-zinc-800/60 px-1.5 py-0.5">
              {entry.category} · {(entry.confidence * 100).toFixed(0)}%
            </span>
            <span className="rounded bg-zinc-800/60 px-1.5 py-0.5">
              {entry.member_count.toLocaleString()} msgs
            </span>
            {entry.oldest_message_age_days !== null ? (
              <span className="rounded bg-zinc-800/60 px-1.5 py-0.5">
                oldest {entry.oldest_message_age_days}d
              </span>
            ) : null}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setExpanded((s) => !s)}
          className="shrink-0 rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-200 hover:bg-zinc-700"
        >
          {expanded ? "Hide" : "Expand"}
        </button>
      </div>

      {entry.proposed.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-2">
          {entry.proposed.map((p) => {
            const isStaged = entry.staged_action_types.includes(p.action);
            return (
              <button
                key={p.action}
                type="button"
                onClick={() =>
                  isStaged
                    ? unstage.mutate(p.action)
                    : stage.mutate({ action: p.action, reason: p.reason })
                }
                title={p.reason}
                className={`rounded-md px-3 py-1.5 text-xs font-medium transition ${
                  isStaged
                    ? "bg-emerald-700 text-white hover:bg-emerald-600"
                    : "bg-zinc-800 text-zinc-100 hover:bg-zinc-700"
                }`}
              >
                {isStaged ? `✓ ${p.action}` : p.action}
              </button>
            );
          })}
        </div>
      ) : null}

      {expanded ? (
        <SamplesPanel data={samples.data ?? []} isLoading={samples.isLoading} />
      ) : null}
    </li>
  );
}

function SamplesPanel({
  data,
  isLoading,
}: {
  data: ClusterSampleMessage[];
  isLoading: boolean;
}): JSX.Element {
  if (isLoading) {
    return (
      <p className="mt-3 border-t border-zinc-800 pt-3 text-xs text-zinc-500">
        Loading samples…
      </p>
    );
  }
  if (data.length === 0) {
    return (
      <p className="mt-3 border-t border-zinc-800 pt-3 text-xs text-zinc-500">
        No samples available.
      </p>
    );
  }
  return (
    <ul className="mt-3 space-y-2 border-t border-zinc-800 pt-3 text-xs text-zinc-300">
      {data.map((m) => (
        <li key={m.provider_msg_id}>
          <div className="truncate font-medium">{m.subject ?? "(no subject)"}</div>
          <div className="truncate text-zinc-500">{m.sender ?? ""}</div>
          {m.snippet ? (
            <div className="mt-0.5 line-clamp-2 text-zinc-400">{m.snippet}</div>
          ) : null}
        </li>
      ))}
    </ul>
  );
}
