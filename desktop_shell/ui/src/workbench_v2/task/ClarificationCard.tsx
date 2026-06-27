import { Button, Textarea } from "@fluentui/react-components";
import { SendRegular } from "@fluentui/react-icons";
import { useState } from "react";
import type { ContainerMessage } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";

interface ClarificationCardProps {
  busy?: boolean;
  message: ContainerMessage;
  onSubmit(taskId: string, input: string): void;
}

export function ClarificationCard({ busy, message, onSubmit }: ClarificationCardProps) {
  const t = useI18n();
  const [input, setInput] = useState("");
  const taskId = message.task_id || message.job_id || "";
  const trimmed = input.trim();

  return (
    <div className="sn-clarification-card" role="group" aria-label={t("clarification.aria")}>
      <Textarea
        className="sn-clarification-input"
        disabled={busy || !taskId}
        placeholder={t("clarification.placeholder")}
        resize="vertical"
        value={input}
        onChange={(_, data) => setInput(data.value)}
      />
      <Button
        appearance="primary"
        disabled={busy || !taskId || !trimmed}
        icon={<SendRegular />}
        onClick={() => {
          if (!taskId || !trimmed) return;
          onSubmit(taskId, trimmed);
          setInput("");
        }}
      >
        {t("clarification.continue")}
      </Button>
    </div>
  );
}
