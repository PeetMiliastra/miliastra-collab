import { useState } from "react";
import {
  Box,
  Boxes,
  Camera,
  FileBox,
  Mountain,
  Search,
  Settings2,
  Backpack,
  SlidersHorizontal,
  TrendingUp,
  CloudSun,
  Coins,
  Crosshair,
  Ghost,
  Hand,
  HeartPulse,
  Package,
  Shield,
  Sparkles,
  Swords,
  Tag,
  Workflow,
  UserRound,
} from "lucide-react";
import type { Asset, Section } from "./types";

const categories = [
  { kind: "Prefab", label: "Prefabs", group: "Objects", icon: Boxes },
  { kind: "Entity", label: "In-world objects", group: "Objects", icon: Box },
  { kind: "Creation", label: "Creations", group: "Objects", icon: Ghost },
  { kind: "Loot", label: "Loot", group: "Objects", icon: Coins },
  { kind: "Terrain", label: "Terrain", group: "Scene editor", icon: Mountain },
  { kind: "Camera", label: "Cameras", group: "Scene editor", icon: Camera },
  {
    kind: "Environment",
    label: "Environment",
    group: "Scene editor",
    icon: CloudSun,
  },
  {
    kind: "Player Template",
    label: "Player Templates",
    group: "Combat",
    icon: UserRound,
  },
  {
    kind: "Class",
    label: "Class",
    group: "Combat",
    icon: UserRound,
  },
  {
    kind: "Growth Curve",
    label: "Growth Curve",
    group: "Combat",
    icon: TrendingUp,
  },
  {
    kind: "Attribute Group",
    label: "Attribute Group",
    group: "Combat",
    icon: SlidersHorizontal,
  },
  {
    kind: "Character Skill",
    label: "Character Skill",
    group: "Combat",
    icon: Swords,
  },
  {
    kind: "Unit Status",
    label: "Unit Status",
    group: "Combat",
    icon: HeartPulse,
  },
  { kind: "Projectile", label: "Projectile", group: "Combat", icon: Crosshair },
  { kind: "VFX Tool", label: "VFX Tool", group: "Combat", icon: Sparkles },
  {
    kind: "Character Control Skill",
    label: "Character Control Skill",
    group: "Combat",
    icon: Hand,
  },
  { kind: "Shield", label: "Shield", group: "Combat", icon: Shield },
  { kind: "Item", label: "Item", group: "Items", icon: Package },
  {
    kind: "Inventory Template",
    label: "Inventory Template",
    group: "Items",
    icon: Backpack,
  },
  {
    kind: "Scan Tag Template",
    label: "Scan Tag Template",
    group: "Items",
    icon: Tag,
  },
  { kind: "Node graph", label: "Node graphs", group: "Logic", icon: Workflow },
  {
    kind: "Configuration",
    label: "Other",
    group: "Other",
    icon: Settings2,
  },
];

export function AssetBrowser({
  slices,
  source,
  assets,
  selected,
  busy,
  onSourceChange,
  onSelect,
}: {
  slices: Section[];
  source: string;
  assets: Asset[] | null;
  selected: string;
  busy: boolean;
  onSourceChange: (source: string) => void;
  onSelect: (key: string) => void;
}) {
  const [category, setCategory] = useState("");
  const [query, setQuery] = useState("");
  const known = new Set(categories.map((item) => item.kind));
  const kinds = [
    ...categories,
    ...Array.from(new Set(assets?.map((asset) => asset.kind)))
      .filter((kind) => !known.has(kind))
      .sort()
      .map((kind) => ({ kind, label: kind, group: "Other", icon: FileBox })),
  ];
  const groups = kinds
    .map((kind) => ({
      ...kind,
      assets: assets?.filter((asset) => asset.kind === kind.kind) ?? [],
    }))
    .filter((kind) => kind.assets.length);
  const search = query.trim().toLocaleLowerCase();
  const visible = groups
    .filter((group) => !category || group.kind === category)
    .map((group) => ({
      ...group,
      assets: group.assets.filter((asset) =>
        `${asset.name} ${asset.kind} ${group.label} ${asset.id}`
          .toLocaleLowerCase()
          .includes(search),
      ),
    }))
    .filter((group) => group.assets.length);
  const visibleCount = visible.reduce(
    (count, group) => count + group.assets.length,
    0,
  );

  return (
    <div className="asset-browser">
      <aside className="asset-browser-sidebar">
        <label className="field">
          From slice
          <select
            aria-label="From slice"
            value={source}
            disabled={busy}
            onChange={(event) => onSourceChange(event.target.value)}
          >
            {slices.map((slice) => (
              <option key={slice.id} value={slice.id}>
                {slice.name} (slice {Number(slice.id) + 1})
              </option>
            ))}
          </select>
        </label>
        <nav className="asset-categories" aria-label="Asset categories">
          <button
            className="asset-category"
            aria-pressed={!category}
            disabled={busy || !assets}
            onClick={() => {
              setCategory("");
              onSelect("");
            }}
          >
            All assets <span aria-hidden="true">{assets?.length ?? 0}</span>
          </button>
          {Array.from(new Set(groups.map((group) => group.group))).map(
            (name) => (
              <div key={name}>
                <h3>{name}</h3>
                {groups
                  .filter((group) => group.group === name)
                  .map((group) => (
                    <button
                      key={group.kind}
                      className="asset-category"
                      aria-pressed={category === group.kind}
                      disabled={busy}
                      onClick={() => {
                        setCategory(group.kind);
                        onSelect("");
                      }}
                    >
                      {group.label}{" "}
                      <span aria-hidden="true">{group.assets.length}</span>
                    </button>
                  ))}
              </div>
            ),
          )}
        </nav>
      </aside>
      <section className="asset-browser-main" aria-label="Assets">
        <div className="asset-browser-toolbar">
          <label className="asset-search">
            <Search size={16} aria-hidden="true" />
            <input
              aria-label="Search assets"
              placeholder="Search assets"
              value={query}
              disabled={busy || !assets}
              onChange={(event) => {
                setQuery(event.target.value);
                onSelect("");
              }}
              type="search"
            />
          </label>
          <span className="asset-count" role="status">
            {assets
              ? `${visibleCount} ${visibleCount === 1 ? "asset" : "assets"}`
              : ""}
          </span>
        </div>
        <div className="asset-browser-results">
          {!assets && (
            <p className="asset-empty" role="status">
              {source ? "Reading saved assets…" : "No slices available."}
            </p>
          )}
          {assets && !assets.length && (
            <p className="asset-empty">
              No named assets were found. Save the stage first if the asset is
              new.
            </p>
          )}
          {assets && assets.length > 0 && !visible.length && (
            <p className="asset-empty">No assets match your search.</p>
          )}
          {visible.map((group) => {
            const Icon = group.icon;
            return (
              <section
                className="asset-kind-group"
                key={group.kind}
                aria-label={group.label}
              >
                <h3>
                  {group.group === group.label
                    ? group.label
                    : `${group.group}: ${group.label}`}
                </h3>
                <ul className="asset-grid">
                  {group.assets.map((asset) => (
                    <li key={asset.key}>
                      <button
                        className="asset-card"
                        aria-label={`${asset.name} — ${asset.kind} (${asset.id})${asset.canGenerateId ? "" : " — Unrecognized ID layout"}`}
                        aria-pressed={selected === asset.key}
                        disabled={busy || !asset.canGenerateId}
                        title={asset.name}
                        onClick={() => onSelect(asset.key)}
                      >
                        <Icon size={28} aria-hidden="true" />
                        <span className="asset-card-name">{asset.name}</span>
                        <small>{asset.id}</small>
                        {!asset.canGenerateId && (
                          <small>Unrecognized ID layout</small>
                        )}
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      </section>
    </div>
  );
}
