import { useEffect, useMemo, useRef, useState } from "react";
import { defaultTauriGateway } from "../infra/runtimePorts";
import type { HistoryCursor, HistoryItem } from "../types";
import { IconBookOpen } from "../ui/icons";
import { PixelButton } from "../ui/PixelButton";

type Props = {
  epoch: number;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
};

const PAGE = 50;

export function HistoryScreen({
  epoch,
  pushToast,
}: Props) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const loadingRef = useRef(false);
  const requestGenerationRef = useRef(0);

  const cursor = useMemo<HistoryCursor | null>(() => {
    if (!items.length) return null;
    const oldest = items[items.length - 1]!;
    return {
      created_at_ms: oldest.created_at_ms,
      task_id: oldest.task_id,
    };
  }, [items]);

  async function loadFirst() {
    const generation = ++requestGenerationRef.current;
    loadingRef.current = true;
    setLoading(true);
    setLoadError(false);
    setHasMore(true);
    setItems([]);
    try {
      const rows = (await defaultTauriGateway.invoke("history_list", {
        limit: PAGE,
        cursor: null,
      })) as HistoryItem[];
      if (generation !== requestGenerationRef.current) return;
      setItems(rows);
      setHasMore(rows.length === PAGE);
      // reset scroll to top when reloading
      scrollerRef.current?.scrollTo({ top: 0 });
    } catch {
      if (generation === requestGenerationRef.current) setLoadError(true);
    } finally {
      if (generation === requestGenerationRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }

  async function loadMore() {
    if (loadingRef.current || loading) return;
    if (!hasMore) return;
    if (cursor == null) return;
    const generation = requestGenerationRef.current;
    loadingRef.current = true;
    setLoading(true);
    try {
      const rows = (await defaultTauriGateway.invoke("history_list", {
        limit: PAGE,
        cursor,
      })) as HistoryItem[];
      if (generation !== requestGenerationRef.current) return;
      setItems((prev) => [...prev, ...rows]);
      setHasMore(rows.length === PAGE);
    } catch {
      if (generation === requestGenerationRef.current) {
        pushToast("Couldn't load more history.", "danger");
      }
    } finally {
      if (generation === requestGenerationRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }

  useEffect(() => {
    void loadFirst();
    return () => {
      requestGenerationRef.current += 1;
      loadingRef.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [epoch]);

  function onScroll() {
    const el = scrollerRef.current;
    if (!el) return;
    const remaining = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (remaining < 140) loadMore();
  }

  async function copyHistoryText(text: string) {
    const value = text.trim();
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      pushToast("Copied", "ok");
    } catch {
      pushToast("Copy failed", "danger");
    }
  }

  return (
    <div className="pageSurface historySurface">
      <header className="pageHeader historyHeader">
        <h1 className="pageTitle">History</h1>
        {!loadError && (!loading || items.length > 0) ? (
          <div className="itemCount" aria-label={`${items.length} loaded items`}>
            <strong>{items.length}</strong>
            <span>loaded</span>
          </div>
        ) : null}
      </header>

      <div
        className="historyScroller"
        ref={scrollerRef}
        onScroll={onScroll}
        aria-busy={loading}
      >
        {loading && items.length === 0 ? (
          <div className="historyEmpty historyState historyLoading" role="status">
            <strong>Loading history...</strong>
            <span>Your saved transcripts will appear shortly.</span>
          </div>
        ) : loadError ? (
          <div className="historyEmpty historyState historyError" role="alert">
            <strong>History couldn't load</strong>
            <span>Try again in a moment.</span>
            <PixelButton className="historyRetry" onClick={() => void loadFirst()}>
              Retry
            </PixelButton>
          </div>
        ) : items.length === 0 ? (
          <div className="historyEmpty historyState">
            <div className="historyEmptyIcon" aria-hidden="true">
              <IconBookOpen size={28} tone="muted" />
            </div>
            <strong>No transcripts yet</strong>
            <span>Completed recordings will appear here.</span>
          </div>
        ) : null}

        {!loadError && items.map((h) => {
          const text = (h.final_text || h.asr_text || "").trim();
          const created = new Date(h.created_at_ms);
          return (
            <button
              type="button"
              key={h.task_id}
              className="historyRow"
              title="Copy to clipboard"
              onClick={() => void copyHistoryText(text)}
            >
              <time className="historyTime" dateTime={created.toISOString()}>
                <strong>{created.toLocaleDateString(undefined, { month: "short", day: "numeric" })}</strong>
                <span>{created.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}</span>
              </time>
              <div className="historyPreview">
                {text || "Empty transcript"}
              </div>
              <span className="historyCopyCue" aria-hidden="true">Copy</span>
            </button>
          );
        })}

        {!loadError && items.length > 0 ? (
          <div className="historyFooter">
            {loading ? "Loading more..." : hasMore ? "More items below" : "All items loaded"}
          </div>
        ) : null}
      </div>
    </div>
  );
}
