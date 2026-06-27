import { useCallback, useEffect, useRef, useState } from "react";

const BOTTOM_THRESHOLD_PX = 56;

export function useStickyScroll(dependencies: readonly unknown[]) {
  const streamRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const frameRef = useRef<number | null>(null);
  const [isPinnedToBottom, setIsPinnedToBottom] = useState(true);

  const updatePinnedState = useCallback(() => {
    const stream = streamRef.current;
    if (!stream) return;
    const nextPinned = isNearScrollBottom(stream);
    pinnedRef.current = nextPinned;
    setIsPinnedToBottom(nextPinned);
  }, []);

  const scrollToBottomIfPinned = useCallback(() => {
    const stream = streamRef.current;
    if (!stream || !pinnedRef.current) return;
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
    }
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      const latestStream = streamRef.current;
      if (!latestStream || !pinnedRef.current) return;
      latestStream.scrollTop = scrollTopForBottom(latestStream);
      updatePinnedState();
    });
  }, [updatePinnedState]);

  const jumpToLatest = useCallback(() => {
    const stream = streamRef.current;
    if (!stream) return;
    pinnedRef.current = true;
    setIsPinnedToBottom(true);
    stream.scrollTop = scrollTopForBottom(stream);
    scrollToBottomIfPinned();
  }, [scrollToBottomIfPinned]);

  const detachFromBottom = useCallback(() => {
    if (!pinnedRef.current) return;
    pinnedRef.current = false;
    setIsPinnedToBottom(false);
  }, []);

  const handleWheelIntent = useCallback(
    (event: WheelEvent) => {
      const stream = streamRef.current;
      if (event.deltaY < -1 && stream && stream.scrollTop > 0) detachFromBottom();
    },
    [detachFromBottom]
  );

  useEffect(() => {
    scrollToBottomIfPinned();
    // The dependency list is supplied by each stream component because the
    // relevant counters differ between Chat and Task.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, dependencies);

  useEffect(() => {
    const stream = streamRef.current;
    if (!stream) return;

    let resizeObserver: ResizeObserver | null = null;
    const observeResizeTargets = () => {
      if (!resizeObserver) return;
      resizeObserver.disconnect();
      resizeObserver.observe(stream);
      Array.from(stream.children).forEach((child) => resizeObserver?.observe(child));
    };

    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => scrollToBottomIfPinned());
      observeResizeTargets();
    }

    const mutationObserver = new MutationObserver(() => {
      observeResizeTargets();
      scrollToBottomIfPinned();
    });
    mutationObserver.observe(stream, {
      characterData: true,
      childList: true,
      subtree: true,
    });

    stream.addEventListener("wheel", handleWheelIntent, { capture: true, passive: true });

    return () => {
      stream.removeEventListener("wheel", handleWheelIntent, { capture: true });
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [handleWheelIntent, scrollToBottomIfPinned]);

  return {
    isPinnedToBottom,
    jumpToLatest,
    streamRef,
    updatePinnedState,
  };
}

export function isNearScrollBottom(
  stream: Pick<HTMLDivElement, "scrollHeight" | "scrollTop" | "clientHeight">
) {
  return distanceFromScrollBottom(stream) <= BOTTOM_THRESHOLD_PX;
}

export function scrollTopForBottom(stream: Pick<HTMLDivElement, "scrollHeight" | "clientHeight">) {
  return Math.max(0, stream.scrollHeight - stream.clientHeight);
}

function distanceFromScrollBottom(
  stream: Pick<HTMLDivElement, "scrollHeight" | "scrollTop" | "clientHeight">
) {
  return Math.max(0, stream.scrollHeight - stream.scrollTop - stream.clientHeight);
}
