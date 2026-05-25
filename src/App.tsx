import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { tauriApi, type AccountRecord } from "./lib/tauri";

export function App(): JSX.Element {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-8 bg-zinc-950 px-6 py-12 text-zinc-100">
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
    queryKey: ["accounts"],
    queryFn: tauriApi.gmailListAccounts,
  });

  const connect = useMutation({
    mutationFn: tauriApi.gmailConnectAccount,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accounts"] });
    },
  });

  return (
    <section className="w-full max-w-md rounded-lg border border-zinc-800 bg-zinc-900/60 p-6 shadow-xl">
      <h2 className="text-lg font-semibold">Connect an account</h2>
      <p className="mt-1 text-xs text-zinc-400">
        Authorize Chhanni to read your Gmail. The connection runs locally; only
        your browser sees the consent screen.
      </p>

      <button
        type="button"
        onClick={() => connect.mutate()}
        disabled={connect.isPending}
        className="mt-4 w-full rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-emerald-500 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {connect.isPending ? "Waiting for browser…" : "Connect Gmail"}
      </button>

      {connect.isError ? (
        <p className="mt-3 text-xs text-red-400">
          {connect.error instanceof Error ? connect.error.message : "Connection failed"}
        </p>
      ) : null}

      <div className="mt-6 border-t border-zinc-800 pt-4">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-zinc-500">
          Connected accounts
        </h3>
        <AccountList
          data={accountsQuery.data ?? []}
          isLoading={accountsQuery.isLoading}
        />
      </div>
    </section>
  );
}

interface AccountListProps {
  data: AccountRecord[];
  isLoading: boolean;
}

function AccountList({ data, isLoading }: AccountListProps): JSX.Element {
  if (isLoading) {
    return <p className="mt-2 text-xs text-zinc-500">Loading…</p>;
  }
  if (data.length === 0) {
    return <p className="mt-2 text-xs text-zinc-500">No accounts connected yet.</p>;
  }
  return (
    <ul className="mt-2 space-y-2">
      {data.map((account) => (
        <li
          key={account.account_id}
          className="flex items-center justify-between rounded border border-zinc-800 bg-zinc-950 px-3 py-2 text-sm"
        >
          <span>{account.email}</span>
          <span className="text-xs uppercase tracking-wider text-zinc-500">
            {account.provider}
          </span>
        </li>
      ))}
    </ul>
  );
}
