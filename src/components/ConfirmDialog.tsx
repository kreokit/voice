import { Button, Dialog } from "../ui";

/** A yes/no question before something that deletes KreoKit's data. */
export function ConfirmDialog({
  title,
  text,
  confirmLabel,
  onConfirm,
  onClose,
}: {
  title: string;
  text: string;
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={title}
      onClose={onClose}
      width={420}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              onClose();
              onConfirm();
            }}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <p className="dialog-text">{text}</p>
    </Dialog>
  );
}

/** The text of "Remove from Recent?" (the user's wording, 2026-10-22). */
export const REMOVE_TEXT =
  "Your file is not touched. KreoKit's preview, waveform and settings for it are deleted; opening the file again starts fresh.";
export const CLEAR_TEXT =
  "Your files are not touched. KreoKit's previews, waveforms and settings for them are deleted.";
