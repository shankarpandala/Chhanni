import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorMessage, tauriApi, type OAuthProviderTag } from "./lib/tauri";

export function Settings(): JSX.Element {
  const status = useQuery({
    queryKey: ["oauthStatus"],
    queryFn: tauriApi.oauthStatus,
  });

  return (
    <section className="mt-8 w-full max-w-2xl space-y-6">
      <header>
        <h2 className="text-lg font-semibold">OAuth credentials</h2>
        <p className="mt-1 text-xs text-zinc-400">
          Chhanni needs an OAuth client ID for each mail provider you want to
          connect. These are registered once on the provider's developer
          console and stored in your OS keychain. Nothing leaves your machine.
        </p>
      </header>

      <ProviderCard
        provider="gmail"
        title="Gmail"
        registrationHelp={
          <>
            Create a <strong>Desktop application</strong> OAuth client at{" "}
            <code className="rounded bg-zinc-800 px-1 py-0.5">
              console.cloud.google.com → APIs &amp; Services → Credentials
            </code>
            . Enable the Gmail API for your project. Copy the Client ID (and
            secret if Google issues one).
          </>
        }
        configured={status.data?.gmail_configured ?? false}
        accentClass="bg-emerald-600 hover:bg-emerald-500"
      />

      <ProviderCard
        provider="graph"
        title="Outlook / Microsoft 365"
        registrationHelp={
          <>
            Create an <strong>App registration</strong> at{" "}
            <code className="rounded bg-zinc-800 px-1 py-0.5">
              entra.microsoft.com → App registrations
            </code>
            . Pick <em>Mobile and desktop</em>, redirect URI{" "}
            <code className="rounded bg-zinc-800 px-1 py-0.5">
              http://localhost
            </code>
            , grant delegated permissions <code>Mail.ReadWrite</code>,{" "}
            <code>offline_access</code>, <code>User.Read</code>. Public clients
            don&apos;t need a secret.
          </>
        }
        configured={status.data?.graph_configured ?? false}
        accentClass="bg-sky-600 hover:bg-sky-500"
      />
    </section>
  );
}

interface ProviderCardProps {
  provider: OAuthProviderTag;
  title: string;
  registrationHelp: React.ReactNode;
  configured: boolean;
  accentClass: string;
}

function ProviderCard({
  provider,
  title,
  registrationHelp,
  configured,
  accentClass,
}: ProviderCardProps): JSX.Element {
  const queryClient = useQueryClient();
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [editing, setEditing] = useState(false);

  const save = useMutation({
    mutationFn: () =>
      tauriApi.setOAuthCredentials(
        provider,
        clientId,
        clientSecret.length > 0 ? clientSecret : null,
      ),
    onSuccess: () => {
      setEditing(false);
      setClientId("");
      setClientSecret("");
      void queryClient.invalidateQueries({ queryKey: ["oauthStatus"] });
    },
  });

  const clear = useMutation({
    mutationFn: () => tauriApi.clearOAuthCredentials(provider),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["oauthStatus"] });
    },
  });

  return (
    <article className="rounded-lg border border-zinc-800 bg-zinc-900/60 p-5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-medium">{title}</h3>
          <p className="mt-1 text-xs text-zinc-400">{registrationHelp}</p>
        </div>
        <span
          className={`shrink-0 rounded px-2 py-1 text-[10px] font-medium uppercase tracking-wider ${
            configured ? "bg-emerald-800/60 text-emerald-200" : "bg-zinc-800 text-zinc-400"
          }`}
        >
          {configured ? "configured" : "not configured"}
        </span>
      </div>

      {editing || !configured ? (
        <div className="mt-4 space-y-2">
          <label className="block text-xs text-zinc-400">
            Client ID
            <input
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={clientId}
              onChange={(e) => setClientId(e.target.value)}
              placeholder={
                provider === "gmail"
                  ? "1234567890-xxx.apps.googleusercontent.com"
                  : "00000000-0000-0000-0000-000000000000"
              }
              className="mt-1 block w-full rounded border border-zinc-800 bg-zinc-950 px-2 py-1.5 text-xs font-mono text-zinc-100"
            />
          </label>
          <label className="block text-xs text-zinc-400">
            Client secret <span className="text-zinc-600">(optional)</span>
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={clientSecret}
              onChange={(e) => setClientSecret(e.target.value)}
              className="mt-1 block w-full rounded border border-zinc-800 bg-zinc-950 px-2 py-1.5 text-xs font-mono text-zinc-100"
            />
          </label>
          <div className="flex items-center gap-2 pt-1">
            <button
              type="button"
              onClick={() => save.mutate()}
              disabled={clientId.trim().length === 0 || save.isPending}
              className={`rounded-md px-3 py-1.5 text-xs font-medium text-white transition disabled:cursor-not-allowed disabled:opacity-50 ${accentClass}`}
            >
              {save.isPending ? "Saving…" : "Save to keychain"}
            </button>
            {configured ? (
              <button
                type="button"
                onClick={() => setEditing(false)}
                className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-200 hover:bg-zinc-700"
              >
                Cancel
              </button>
            ) : null}
          </div>
          {save.isError ? (
            <p className="text-xs text-red-400">
              {errorMessage(save.error, "Failed to save credentials")}
            </p>
          ) : null}
        </div>
      ) : (
        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() => setEditing(true)}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-zinc-200 hover:bg-zinc-700"
          >
            Replace
          </button>
          <button
            type="button"
            onClick={() => clear.mutate()}
            disabled={clear.isPending}
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-xs font-medium text-red-300 hover:bg-red-900/40 disabled:opacity-50"
          >
            Remove
          </button>
        </div>
      )}
    </article>
  );
}
