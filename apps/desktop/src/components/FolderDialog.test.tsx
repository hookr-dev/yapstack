import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FolderDialog } from "./FolderDialog";

describe("FolderDialog", () => {
  const defaultProps = {
    open: true,
    onOpenChange: vi.fn(),
    mode: "create" as const,
    onSubmit: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows 'New Folder' title in create mode", () => {
    render(<FolderDialog {...defaultProps} mode="create" />);
    expect(screen.getByText("New Folder")).toBeInTheDocument();
  });

  it("shows 'Edit Folder' title in edit mode", () => {
    render(<FolderDialog {...defaultProps} mode="edit" />);
    expect(screen.getByText("Edit Folder")).toBeInTheDocument();
  });

  it("calls onSubmit with valid name", async () => {
    const onSubmit = vi.fn();
    render(<FolderDialog {...defaultProps} onSubmit={onSubmit} />);
    const input = screen.getByPlaceholderText("Folder name");
    await userEvent.type(input, "My Folder");
    await userEvent.click(screen.getByText("Create"));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ name: "My Folder" }),
    );
  });

  it("disables submit button when name is empty", () => {
    render(<FolderDialog {...defaultProps} />);
    const createButton = screen.getByText("Create");
    expect(createButton).toBeDisabled();
  });

  it("pre-populates fields from initialData", () => {
    render(
      <FolderDialog
        {...defaultProps}
        mode="edit"
        initialData={{ name: "Existing", description: "A description" }}
      />,
    );
    const input = screen.getByPlaceholderText("Folder name");
    expect(input).toHaveValue("Existing");
    const textarea = screen.getByPlaceholderText("Add context for AI chat...");
    expect(textarea).toHaveValue("A description");
  });

  it("shows parent context in create mode", () => {
    render(
      <FolderDialog {...defaultProps} parentName="Parent Folder" />,
    );
    expect(screen.getByText("Parent Folder")).toBeInTheDocument();
    expect(screen.getByText(/Creating inside/)).toBeInTheDocument();
  });

  it("shows Save button in edit mode", () => {
    render(
      <FolderDialog
        {...defaultProps}
        mode="edit"
        initialData={{ name: "Test" }}
      />,
    );
    expect(screen.getByText("Save")).toBeInTheDocument();
  });
});
