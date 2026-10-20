import { isCloudProvider, providerName } from "./providers";
import { useState } from "react";
import { call } from "./bridge";
import { Confirm } from "./components";
import type { Member, Run, Snapshot } from "./types";

/** The key goes in the message text, never in the invitation link. */
export function invitationMessage(project: string, link: string, key: string) {
  return [
    "Join my Miliastra project “" + project + "” on miliastra/collab.",
    "",
    "Download miliastra/collab on https://miliastra.dev",
    "Open this link on your Windows PC: " + link,
    "It works for 7 days.",
    "",
    "Encryption Key: " + key,
    "Paste the Encryption Key when the app asks for it.",
  ].join("\n");
}

/** Team members, with Remove and Make owner for the owner. */
export function TeamPanel({
  state,
  busy,
  run,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
}) {
  const [asking, setAsking] = useState<{
    kind: "owner" | "remove";
    member: Member;
  } | null>(null);
  const project = state.project;
  if (!project) return null;
  const owner = project.members.some((m) => m.id === state.member && m.owner);
  const folder = isCloudProvider(project.storage.kind)
    ? providerName(project.storage.kind)
    : "the shared folder";
  const others = project.members.filter((m) => m.id !== state.member);
  return (
    <div className="settings-card team-panel">
      <h2>Team</h2>
      <ul className="team-list" aria-label="Members">
        {[...project.members]
          .sort(
            (a, b) =>
              Number(b.id === state.member) - Number(a.id === state.member),
          )
          .map((m) => (
            <li key={m.id}>
              <span>
                <strong>
                  {m.id === state.member
                    ? m.name === "You"
                      ? "You"
                      : `${m.name} (you)`
                    : m.name}
                </strong>
                {m.owner && <small>owner</small>}
              </span>
              {owner && m.id !== state.member && (
                <span className="team-actions">
                  <button
                    className="text-button"
                    disabled={!!busy || !state.online}
                    onClick={() => setAsking({ kind: "owner", member: m })}
                  >
                    Make owner
                  </button>
                  <button
                    className="text-button danger"
                    disabled={!!busy || !state.online}
                    onClick={() => setAsking({ kind: "remove", member: m })}
                  >
                    Remove
                  </button>
                </span>
              )}
            </li>
          ))}
      </ul>
      {!owner && others.length > 0 && (
        <p className="muted">
          The owner can remove creators or hand over ownership.
        </p>
      )}
      {asking?.kind === "owner" && (
        <Confirm
          title={`Make ${asking.member.name} the owner?`}
          confirmLabel="Make owner"
          busy={!!busy}
          onClose={() => setAsking(null)}
          onConfirm={() => {
            const m = asking.member;
            setAsking(null);
            void run(`Making ${m.name} the owner`, () =>
              call("transfer_ownership", { member: m.id }),
            );
          }}
        >
          <p>
            {asking.member.name} will let new creators into the team folder and
            can remove creators. You stay a member.
          </p>
          <p>
            The team folder stays in your{" "}
            {folder === "the shared folder" ? "storage" : folder}, so don’t
            delete it.
          </p>
        </Confirm>
      )}
      {asking?.kind === "remove" && (
        <Confirm
          title={`Remove ${asking.member.name} from ${project.name}?`}
          confirmLabel="Remove"
          danger
          busy={!!busy}
          onClose={() => setAsking(null)}
          onConfirm={() => {
            const m = asking.member;
            setAsking(null);
            void run(`Removing ${m.name}`, () =>
              call("remove_member", { member: m.id }),
            );
          }}
        >
          <p>
            They can no longer reserve or share slices. Their reservations are
            released and invitations they created stop working. Their shared
            updates stay in History.
          </p>
          <p>
            Also stop sharing the team folder with them in {folder}. They keep
            any files and the Encryption Key they already have.
          </p>
        </Confirm>
      )}
    </div>
  );
}
