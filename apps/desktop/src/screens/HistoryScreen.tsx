import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { copyText } from "../lib/clipboard";
import type { HistoryItem } from "../types";

type Props = {
  epoch: number;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
};

const PAGE = 50;

export function HistoryScreen({ epoch, pushToast }: Props) {
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
      const rows = (await invoke("history_list", {
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
      const rows = (await invoke("history_list", {
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

  async function copyItem(h: HistoryItem) {
    const text = (h.final_text || h.asr_text || "").trim();
    if (!text) return;
    try {
      await copyText(text);
      pushToast("COPIED", "ok");
    } catch {
      pushToast("COPY FAILED", "danger");
    }
  }

  return (
    <div className="card">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <div className="sectionTitle" style={{ margin: 0 }}>
          HISTORY
        </div>
        <div className="muted">{items.length}</div>
      </div>

      <div className="historyScroller" ref={scrollerRef} onScroll={onScroll}>
        {items.map((h) => (
          <div key={h.task_id} className="historyRow" tabIndex={0}>
            <div className="historyTime">
              {new Date(h.created_at_ms).toLocaleString()}
            </div>
            <div className="historyPreview">
              {(h.final_text || h.asr_text || "").trim() || "-"}
            </div>
            <button
              type="button"
              className="historyCopy"
              onClick={() => copyItem(h)}
              title="COPY"
            >
              COPY
            </button>
          </div>
        ))}

        <div className="historyFooter">
          {loading ? "LOADING..." : hasMore ? "SCROLL" : "END"}
        </div>
      </div>
    </div>
  );
}
