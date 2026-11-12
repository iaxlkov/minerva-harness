// assistant-ui attachment element: https://www.assistant-ui.com/docs/ui/attachment
// Adapted to this project's Tailwind 3 prefix, Radix Dialog and Tooltip; no Avatar or shadcn dialog wrapper.
import {useEffect, useState, type FC} from "react";
import {AttachmentPrimitive, ComposerPrimitive, useAui, useAuiState} from "@assistant-ui/react";
import {ImageEditor} from "../../../harness/ImageEditor";
import {AlertCircleIcon, FileText, Loader2Icon, XIcon} from "lucide-react";
import {Tooltip, TooltipContent, TooltipTrigger} from "../../ui/tooltip";
import {cn} from "../../../lib/utils";

/** Object URL for an image attachment that still holds its File (composer side). */
function useAttachmentSrc() {
  const file = useAuiState(s => (s.attachment.type === "image" ? s.attachment.file : undefined));
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    if (!file) { setSrc(undefined); return; }
    const url = URL.createObjectURL(file);
    setSrc(url);
    return () => URL.revokeObjectURL(url);
  }, [file]);
  return src;
}

const AttachmentTile: FC = () => {
  const src = useAttachmentSrc();
  const aui = useAui();
  const file = useAuiState(s => (s.attachment.type === "image" ? s.attachment.file : undefined));
  const [editing, setEditing] = useState(false);
  // The edited PNG replaces this attachment; pin comments are appended to the message text.
  const applyEdit = async (edited: File, comments: string) => {
    await aui.composer().addAttachment(edited);
    await aui.attachment().remove();
    if (comments) { const text = aui.composer().getState().text; aui.composer().setText(text ? `${text}\n\n${comments}` : comments); }
  };
  const name = useAuiState(s => s.attachment.name);
  const isImage = useAuiState(s => s.attachment.type === "image");
  const extension = name.includes(".") ? name.split(".").pop()!.toUpperCase().slice(0, 6) : "ФАЙЛ";
  const state = useAuiState(s => s.attachment.status.type === "incomplete" && s.attachment.status.reason === "error" ? "error" : s.attachment.status.type === "running" ? "uploading" : undefined);
  const tile = (
    <div
      role={src ? "button" : "group"}
      onClick={src ? () => setEditing(true) : undefined}
      tabIndex={0}
      aria-label={`${src ? "Редактировать" : "Вложение"} ${name}${state === "error" ? ", ошибка загрузки" : state === "uploading" ? ", загружается" : ""}`}
      className={cn(
        "aui-attachment-tile oe-relative oe-flex oe-h-14 oe-items-center oe-overflow-hidden oe-rounded-xl oe-border oe-border-solid oe-bg-background oe-outline-none focus-visible:oe-outline focus-visible:oe-outline-2 focus-visible:oe-outline-ring",
        isImage ? "oe-w-14" : "oe-max-w-56 oe-gap-2.5 oe-pl-2 oe-pr-8",
        state === "error" ? "oe-border-destructive" : "oe-border-border",
        src ? "oe-cursor-zoom-in" : "oe-cursor-default",
      )}
      onKeyDown={src ? e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.currentTarget.click(); } } : undefined}
    >
      {src ? <img src={src} alt="" className="oe-size-full oe-object-cover" />
        : <>
          <span className="oe-grid oe-size-10 oe-shrink-0 oe-place-items-center oe-rounded-lg oe-bg-muted"><FileText className="oe-size-5 oe-text-muted-foreground" /></span>
          <span className="oe-flex oe-min-w-0 oe-flex-col oe-text-xs">
            <span className="oe-truncate oe-text-foreground">{name}</span>
            <span className="oe-text-[11px] oe-text-muted-foreground">{extension}</span>
          </span>
        </>}
      {state && (
        <span aria-hidden="true" className="oe-absolute oe-inset-0 oe-grid oe-place-items-center oe-bg-[color:color-mix(in_srgb,var(--background)_65%,transparent)]">
          {state === "uploading" ? <Loader2Icon className="oe-size-4 oe-animate-spin oe-text-muted-foreground" /> : <AlertCircleIcon className="oe-size-4 oe-text-destructive" />}
        </span>
      )}
    </div>
  );
  return (
    <AttachmentPrimitive.Root className="aui-attachment-root oe-relative oe-shrink-0">
      {file && <ImageEditor file={file} name={name} open={editing} onOpenChange={setEditing} onApply={(edited, comments) => void applyEdit(edited, comments)} />}
      <Tooltip>
        <TooltipTrigger asChild>{tile}</TooltipTrigger>
        <TooltipContent className="oe-studio" side="top"><AttachmentPrimitive.Name /></TooltipContent>
      </Tooltip>
      <AttachmentPrimitive.Remove asChild>
        <button type="button" aria-label={`Удалить ${name}`} className="oe-absolute oe--right-1.5 oe--top-1.5 oe-grid oe-size-5 [box-shadow:0_0_0_2px_var(--secondary)] oe-cursor-pointer oe-place-items-center oe-rounded-full oe-border-0 oe-bg-[#000b] oe-p-0 oe-text-white hover:oe-bg-black focus-visible:oe-outline focus-visible:oe-outline-2 focus-visible:oe-outline-ring">
          <XIcon className="oe-size-3" strokeWidth={2.5} />
        </button>
      </AttachmentPrimitive.Remove>
    </AttachmentPrimitive.Root>
  );
};

export const ComposerAttachments: FC<{className?: string}> = ({className}) => (
  <div className={cn("aui-composer-attachments oe-flex oe-w-full oe-flex-row oe-items-center oe-gap-2 oe-overflow-x-auto empty:oe-hidden", className)}>
    <ComposerPrimitive.Attachments>{() => <AttachmentTile />}</ComposerPrimitive.Attachments>
  </div>
);
