import type { ContainerMessage } from "../../protocol/generated/types";
import { ChatMessageStream } from "./ChatMessageStream";

interface AgentChatSurfaceProps {
  messages: ContainerMessage[];
}

export function AgentChatSurface({ messages }: AgentChatSurfaceProps) {
  return <ChatMessageStream messages={messages} />;
}
