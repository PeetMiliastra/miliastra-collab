import { useSerialWrites } from "./serialWrites";
import { MAX_SLICE_NAME_UNITS } from "./commands.generated";
import { useState } from "react";
import { Check, LoaderCircle, X } from "lucide-react";
import { call } from "./bridge";
import type { Run, Section, Snapshot } from "./types";

export function SliceName({
  state,
  section,
  busy,
  run,
}: {
  state: Snapshot;
  section: Section;
  busy: boolean;
  run: Run;
}) {
  const restored = state.nameSync?.draft;
  const initial = restored?.edits[section.id];
  const [editing, setEditing] = useState(
    !!initial && initial.name !== section.name,
  );
  const [value, setValue] = useState(initial?.name ?? section.name);
  const [savedDraft, setSavedDraft] = useState(
    !!initial && initial.name !== section.name,
  );
  const [version, setVersion] = useState(
    initial?.parentVersion ?? state.project?.namesVersion ?? 0,
  );
  const [baseName, setBaseName] = useState(initial?.baseName ?? section.name);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const writes = useSerialWrites();
  const stale = version !== (state.project?.namesVersion ?? 0);
  const valid =
    !!value.trim() &&
    value.trim().length <= MAX_SLICE_NAME_UNITS &&
    !/[\u0000-\u001f\u007f]/.test(value);
  const persist = (name: string, parentVersion = version) => {
    const saved = writes.enqueue(() =>
      call("save_names_draft", {
        projectId: state.project!.id,
        draft: {
          edits: { [section.id]: { name, baseName, parentVersion } },
        },
      }),
    );
    void saved.catch((e) => setError(String(e)));
    return saved;
  };
  const cancel = () => {
    setEditing(false);
    setValue(section.name);
    setError("");
    setSavedDraft(false);
    // Cancels only this row; the other rows' drafts are kept.
    const saved = writes.enqueue(() =>
      call("save_names_draft", {
        projectId: state.project!.id,
        draft: {
          edits: {
            [section.id]: {
              name: section.name,
              baseName: section.name,
              parentVersion: state.project?.namesVersion ?? 0,
            },
          },
        },
      }),
    );
    void saved.catch((e) => setError(String(e)));
  };
  const save = async () => {
    if (!valid || stale || saving || busy) return;
    setSaving(true);
    setError("");
    try {
      await run(
        state.online ? "Saving slice name" : "Saving name draft",
        async () => {
          await persist(value.trim());
          await writes.flush();
          setValue(value.trim());
          setSavedDraft(true);
          if (state.online) {
            await call("share_section_names", {
              names: { [section.id]: value.trim() },
              parentVersion: version,
            });
            setSavedDraft(false);
          }
          setEditing(false);
        },
        { onError: setError },
      );
    } finally {
      setSaving(false);
    }
  };
  if (section.id === "shared")
    return (
      <strong title="Level-wide data shared by all slices, separate from local editor settings.">
        {section.name}
      </strong>
    );
  if (!editing)
    return (
      <>
        <button
          className="slice-name-button"
          disabled={busy}
          title="Rename slice"
          onClick={() => {
            if (!savedDraft) {
              setValue(section.name);
              setBaseName(section.name);
              setVersion(state.project?.namesVersion ?? 0);
            }
            setError("");
            setEditing(true);
          }}
        >
          {savedDraft ? value : section.name}
        </button>
        {savedDraft && <small className="muted">Draft</small>}
      </>
    );
  return (
    <div className="slice-name-editor">
      <div className="slice-name-controls">
        <input
          autoFocus
          aria-label={`Name for ${section.name}`}
          maxLength={MAX_SLICE_NAME_UNITS}
          value={value}
          disabled={busy || saving}
          onChange={(e) => {
            setValue(e.target.value);
            persist(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void save();
            }
            if (e.key === "Escape" && !saving && !busy) {
              e.preventDefault();
              cancel();
            }
          }}
        />
        <button
          className="icon-button"
          aria-label={`Save name for ${section.name}`}
          title={state.online ? "Share name" : "Save draft"}
          disabled={
            busy || saving || !valid || stale || value.trim() === section.name
          }
          onClick={() => void save()}
        >
          {saving ? (
            <LoaderCircle size={15} className="spin" />
          ) : (
            <Check size={15} />
          )}
        </button>
        <button
          className="icon-button"
          aria-label={`Cancel renaming ${section.name}`}
          disabled={busy || saving}
          onClick={cancel}
        >
          <X size={15} />
        </button>
      </div>
      {stale && (
        <div className="inline-error" role="alert">
          Slice names changed.{" "}
          <button
            className="text-button"
            onClick={() => {
              setVersion(state.project?.namesVersion ?? 0);
              setBaseName(section.name);
              cancel();
              setEditing(true);
            }}
          >
            Use current name
          </button>
        </div>
      )}
      {error && (
        <span className="inline-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
