// assistant-ui Elements conversation search: https://www.assistant-ui.com/elements/conversation-search
// Utility names adapted to this project's Tailwind 3 prefix; the hit list under the bar is a local addition.
import type {ComponentProps, KeyboardEvent, Ref} from "react";
import {ChevronDownIcon, ChevronUpIcon, SearchIcon, XIcon} from "lucide-react";
import {cn} from "../../../lib/utils";
import {field, ghostButton, mono, paper} from "./surfaces";

export interface SearchHit {
  id: string;
  before: string;
  match: string;
  after: string;
  /** Vertical position in the conversation, 0–100. */
  position: number;
}

export function ConversationSearch({
  query,
  hits,
  activeIndex,
  onQueryChange,
  onStep,
  onSelect,
  onClose,
  inputRef,
  className,
  ...props
}: Omit<ComponentProps<"div">, "children" | "onSelect"> & {
  query: string;
  hits: readonly SearchHit[];
  activeIndex: number;
  onQueryChange?: (query: string) => void;
  onStep?: (delta: number) => void;
  onSelect?: (index: number) => void;
  onClose?: () => void;
  inputRef?: Ref<HTMLInputElement>;
}) {
  const index = hits.length === 0 ? -1 : Math.min(Math.max(activeIndex, 0), hits.length - 1);
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") { event.preventDefault(); onStep?.(event.shiftKey ? -1 : 1); }
    else if (event.key === "Escape") { event.preventDefault(); onClose?.(); }
  };
  return (
    <div data-slot="conversation-search" role="search" className={cn("oe-flex oe-w-full oe-max-w-sm oe-gap-2", className)} {...props}>
      <div className="oe-flex oe-min-w-0 oe-flex-1 oe-flex-col oe-gap-2">
        <div className={cn(paper, "oe-flex oe-items-center oe-gap-2 oe-rounded-full oe-py-1.5 oe-pr-1.5 oe-pl-3 oe-shadow-lg")}>
          <SearchIcon className="oe-text-muted-foreground oe-size-3.5 oe-shrink-0" aria-hidden="true" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => onQueryChange?.(event.target.value)}
            onKeyDown={keys}
            placeholder="Найти в диалоге"
            aria-label="Найти в диалоге"
            className="oe-text-foreground placeholder:oe-text-muted-foreground oe-min-w-0 oe-flex-1 oe-border-0 oe-bg-transparent oe-text-[13px] oe-outline-none"
          />
          <span className={cn(mono, "oe-text-muted-foreground oe-shrink-0 oe-tabular-nums")} aria-live="polite">
            {hits.length === 0 ? (query ? "0" : "") : `${index + 1}/${hits.length}`}
          </span>
          {onStep && (
            <>
              <button type="button" aria-label="Предыдущее совпадение" disabled={!hits.length} onClick={() => onStep(-1)} className={cn(ghostButton, "oe-size-6 oe-shrink-0 oe-border-0 oe-bg-transparent oe-p-0 disabled:oe-opacity-40")}>
                <ChevronUpIcon className="oe-size-3.5" />
              </button>
              <button type="button" aria-label="Следующее совпадение" disabled={!hits.length} onClick={() => onStep(1)} className={cn(ghostButton, "oe-size-6 oe-shrink-0 oe-border-0 oe-bg-transparent oe-p-0 disabled:oe-opacity-40")}>
                <ChevronDownIcon className="oe-size-3.5" />
              </button>
            </>
          )}
          {onClose && (
            <button type="button" aria-label="Закрыть поиск" onClick={onClose} className={cn(ghostButton, "oe-size-6 oe-shrink-0 oe-border-0 oe-bg-transparent oe-p-0")}>
              <XIcon className="oe-size-3.5" />
            </button>
          )}
        </div>

        {hits.length > 0 && (
          <ul aria-label="Найденные сообщения" className={cn(paper, "oe-m-0 oe-flex oe-max-h-64 oe-list-none oe-flex-col oe-gap-0.5 oe-overflow-y-auto oe-rounded-xl oe-p-1 oe-shadow-lg")}>
            {hits.map((hit, i) => (
              <li key={hit.id}>
                <button
                  type="button"
                  aria-current={i === index || undefined}
                  onClick={() => onSelect?.(i)}
                  className={cn("oe-w-full oe-cursor-pointer oe-rounded-lg oe-border-0 oe-bg-transparent oe-px-3 oe-py-1.5 oe-text-left oe-text-xs oe-leading-relaxed", i === index && field)}
                >
                  <span className="oe-text-muted-foreground">{hit.before}</span>
                  <mark className="oe-text-foreground oe-rounded oe-bg-[color:color-mix(in_srgb,#f59e0b_35%,transparent)] oe-px-0.5">{hit.match}</mark>
                  <span className="oe-text-muted-foreground">{hit.after}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="oe-relative oe-w-1.5 oe-shrink-0 oe-rounded-full oe-bg-[color:color-mix(in_srgb,var(--foreground)_6%,transparent)]" aria-hidden="true">
        {hits.map((hit, i) => (
          <span
            key={hit.id}
            className={cn("oe-absolute oe-inset-x-0 oe-h-1 oe-rounded-full oe-transition-colors oe-duration-200", i === index ? "oe-bg-[#f59e0b]" : "oe-bg-[color:color-mix(in_srgb,#f59e0b_35%,transparent)]")}
            style={{top: `${hit.position}%`}}
          />
        ))}
      </div>
    </div>
  );
}
