import type { Meta, StoryObj } from "@storybook/react-vite";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PolicyCard } from "./GuardrailPanel";

const meta = {
  title: "Components/GuardrailPanel",
  component: PolicyCard,
} satisfies Meta<typeof PolicyCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const EnforcedRule: Story = {
  args: {
    title: "Block API tokens",
    description: "Built-in detection before requests leave the gateway.",
    enabled: true,
    badges: (
      <>
        <Badge tone="danger">block</Badge>
        <Badge tone="outline">pre-call</Badge>
      </>
    ),
    details: "Includes user and assistant messages",
    actions: <Button variant="outline">Edit rule</Button>,
  },
};
