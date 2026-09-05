import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, within } from "storybook/test";

import { Field } from "./field";
import { Input } from "./input";
import { Select } from "./select";

const meta = {
  title: "Primitives/Field",
  component: Field,
  parameters: { layout: "padded" },
  args: { label: "Model name" },
} satisfies Meta<typeof Field>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
};

export const WithHint: Story = {
  args: { hint: "The alias callers address; rewritten to the upstream name." },
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
};

export const WithError: Story = {
  args: { error: "A model with this name already exists in the project." },
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
};

export const WithInfo: Story = {
  args: {
    info: "Callers send this name; rolter maps it onto the upstream model of the target provider.",
  },
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
};

/**
 * Almost no call site passes `htmlFor`, so the field generates an id and puts
 * it on its single child. That is the whole reason the component exists: before
 * it, every label in the dashboard pointed at nothing and a screen reader
 * announced the control as unlabelled.
 */
export const LabelsItsControl: Story = {
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByLabelText("Model name")).toHaveValue("gpt-4o");
  },
};

/**
 * An error is announced as well as coloured: it takes `role="alert"`, is tied
 * to the control through `aria-describedby`, and flips `aria-invalid`, so the
 * state and the reason arrive together rather than as a control that silently
 * refuses to submit.
 */
export const ErrorDescribesTheControl: Story = {
  args: { error: "A model with this name already exists in the project." },
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const control = canvas.getByLabelText("Model name");
    await expect(control).toHaveAttribute("aria-invalid", "true");
    await expect(control).toHaveAccessibleDescription(/already exists/);
    await expect(canvas.getByRole("alert")).toBeVisible();
  },
};

/** The (i) beside the label opens the note without disturbing the control. */
export const InfoHintOpensOnHover: Story = {
  args: {
    info: "Callers send this name; rolter maps it onto the upstream model of the target provider.",
  },
  render: (args) => (
    <Field {...args}>
      <Input defaultValue="gpt-4o" />
    </Field>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.hover(canvas.getByRole("button", { name: /about model name/i }));
    await expect(canvas.getByRole("tooltip")).toHaveTextContent(/upstream model/);
  },
};

/** A field wraps whatever control it is handed, not just an `Input`. */
export const AroundASelect: Story = {
  args: { label: "Strategy", hint: "How requests are balanced across targets." },
  render: (args) => (
    <Field {...args}>
      <Select defaultValue="round_robin">
        <option value="round_robin">Round robin</option>
        <option value="least_latency">Least latency</option>
      </Select>
    </Field>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByLabelText("Strategy")).toHaveValue("round_robin");
  },
};
