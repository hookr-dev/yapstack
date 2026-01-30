import { Command as CommandPrimitive } from "cmdk"

import * as DialogPrimitive from "@radix-ui/react-dialog"

import { cn } from "@/lib/utils"
import {
  Dialog,
  DialogOverlay,
  DialogPortal,
} from "@/components/ui/dialog"
import { Search } from "lucide-react"

function Command({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive>) {
  return (
    <CommandPrimitive
      data-slot="command"
      className={cn(
        "flex h-full w-full flex-col overflow-hidden rounded-md bg-popover text-popover-foreground",
        className,
      )}
      {...props}
    />
  )
}

function CommandDialog({
  children,
  shouldFilter,
  ...props
}: React.ComponentProps<typeof Dialog> & { shouldFilter?: boolean }) {
  return (
    <Dialog {...props}>
      <DialogPortal>
        <DialogOverlay className="bg-black/50 backdrop-blur-[6px]" />
        <DialogPrimitive.Content
          data-slot="dialog-content"
          className="fixed left-[50%] top-[15%] z-50 grid w-full max-w-[640px] translate-x-[-50%] overflow-hidden rounded-xl border border-border/50 bg-popover p-0 shadow-2xl shadow-black/20 duration-200 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-[0.98] data-[state=open]:zoom-in-[0.98] data-[state=closed]:slide-out-to-top-[2%] data-[state=open]:slide-in-from-top-[2%]"
        >
          <DialogPrimitive.Title className="sr-only">Search</DialogPrimitive.Title>
          <Command shouldFilter={shouldFilter} className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:text-muted-foreground/80 [&_[cmdk-group]]:px-2 [&_[cmdk-item]]:px-3 [&_[cmdk-item]]:py-2.5 [&_[cmdk-input]]:h-12">
            {children}
          </Command>
        </DialogPrimitive.Content>
      </DialogPortal>
    </Dialog>
  )
}

function CommandInput({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Input>) {
  return (
    <div className="flex items-center border-b border-border/50 px-4" data-slot="command-input-wrapper">
      <Search className="mr-3 h-[18px] w-[18px] shrink-0 text-muted-foreground" />
      <CommandPrimitive.Input
        data-slot="command-input"
        className={cn(
          "flex h-12 w-full rounded-md bg-transparent py-3 text-[15px] outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        {...props}
      />
    </div>
  )
}

function CommandList({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.List>) {
  return (
    <CommandPrimitive.List
      data-slot="command-list"
      className={cn("max-h-[min(400px,50vh)] scroll-py-1 overflow-y-auto overflow-x-hidden command-list-scroll", className)}
      {...props}
    />
  )
}

function CommandEmpty({
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Empty>) {
  return (
    <CommandPrimitive.Empty
      data-slot="command-empty"
      className="py-12 text-center text-sm text-muted-foreground"
      {...props}
    />
  )
}

function CommandGroup({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Group>) {
  return (
    <CommandPrimitive.Group
      data-slot="command-group"
      className={cn(
        "overflow-hidden px-2 py-1 text-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-2 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wider [&_[cmdk-group-heading]]:text-muted-foreground/80",
        className,
      )}
      {...props}
    />
  )
}

function CommandItem({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Item>) {
  return (
    <CommandPrimitive.Item
      data-slot="command-item"
      className={cn(
        "relative flex cursor-default gap-3 select-none items-center rounded-lg px-3 py-2.5 text-sm outline-none transition-colors duration-100 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-[18px] [&_svg]:shrink-0",
        className,
      )}
      {...props}
    />
  )
}

function CommandSeparator({
  className,
  ...props
}: React.ComponentProps<typeof CommandPrimitive.Separator>) {
  return (
    <CommandPrimitive.Separator
      data-slot="command-separator"
      className={cn("mx-2 my-1 h-px bg-border/50", className)}
      {...props}
    />
  )
}

function CommandShortcut({
  className,
  ...props
}: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="command-shortcut"
      className={cn(
        "ml-auto inline-flex items-center gap-0.5 text-[11px] font-medium text-muted-foreground/70 keybind-display",
        className,
      )}
      {...props}
    />
  )
}

function CommandFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="command-footer"
      className={cn(
        "flex items-center gap-4 border-t border-border/50 px-4 py-2.5 text-[11px] text-muted-foreground/70",
        className,
      )}
      {...props}
    />
  )
}

function CommandLoading({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="command-loading"
      className={cn("absolute top-0 left-0 right-0 h-[2px] overflow-hidden", className)}
      {...props}
    >
      <div className="h-full w-1/3 bg-primary/40 animate-command-loading rounded-full" />
    </div>
  )
}

export {
  Command,
  CommandDialog,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandSeparator,
  CommandShortcut,
  CommandFooter,
  CommandLoading,
}
