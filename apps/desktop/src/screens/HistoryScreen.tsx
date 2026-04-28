import { useEffect, useMemo, useRef, useState } from "react";
import {
  defaultTauriGateway,
  type TauriGateway,
} from "../infra/runtimePorts";
import type { HistoryItem } from "../types";

type Props = {
  epoch: number;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  gateway?: TauriGateway;
};

const PAGE = 50;

export function HistoryScreen({
  epoch,
  pushToast,
  gateway = defaultTauriGateway,
}: Props) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const scrollerRef = useRef<HTMLDivElement | null>(null);

  const oldestMs = useMemo(() => {
    if (!items.length) return null;
    return items[items.length - 1]!.created_at_ms;
  }, [items]);

  async function loadFirst() {
    setLoading(true);
    setHasMore(true);
    try {
      const rows = (await gateway.invoke("history_list", {
        limit: PAGE,
        beforeMs: null,
      })) as HistoryItem[];
      setItems(rows);
      setHasMore(rows.length === PAGE);
      // reset scroll to top when reloading
      scrollerRef.current?.scrollTo({ top: 0 });
    } catch {
      pushToast("HISTORY LOAD FAILED", "danger");
    } finally {
      setLoading(false);
    }
  }

  async function loadMore() {
    if (loading) return;
    if (!hasMore) return;
    if (oldestMs == null) return;
    setLoading(true);
    try {
      const rows = (await gateway.invoke("history_list", {
        limit: PAGE,
        beforeMs: oldestMs,
      })) as HistoryItem[];
      setItems((prev) => [...prev, ...rows]);
      setHasMore(rows.length === PAGE);
    } catch {
      pushToast("HISTORY LOAD FAILED", "danger");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    loadFirst();
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
      <div className="pageHeader">
        <div className="sectionTitle">history</div>
        <div className="muted">{items.length} items</div>
      </div>

      <div className="historyScroller" ref={scrollerRef} onScroll={onScroll}>
        {items.map((h) => {
          const text = (h.final_text || h.asr_text || "").trim();
          return (
            <div
              key={h.task_id}
              className="historyRow"
              role="button"
              tabIndex={0}
              title="Copy"
              onClick={() => void copyHistoryText(text)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  void copyHistoryText(text);
                }
              }}
            >
              <div className="historyTime">
                {new Date(h.created_at_ms).toLocaleString()}
              </div>
              <div className="historyPreview">
                {text || "-"}
              </div>
            </div>
          );
        })}

        <div className="historyFooter">
          {loading ? "Loading..." : hasMore ? "Scroll" : "End"}
        </div>
      </div>
    </div>
  );
}
