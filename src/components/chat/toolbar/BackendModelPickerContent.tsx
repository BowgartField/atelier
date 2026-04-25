import { Check } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Input } from '@/components/ui/input'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import type { CustomCliProfile } from '@/types/preferences'
import { useAvailableOpencodeModels } from '@/services/opencode-cli'
import { useAvailableCursorModels } from '@/services/cursor-cli'
import { cn } from '@/lib/utils'
import {
  BackendLabel,
  getBackendPlainLabel,
} from '@/components/ui/backend-label'
import {
  formatCursorModelLabel,
  formatOpencodeModelLabel,
  getProviderDisplayName,
} from '@/components/chat/toolbar/toolbar-utils'
import { useToolbarDerivedState } from '@/components/chat/toolbar/useToolbarDerivedState'

interface BackendModelPickerContentProps {
  open: boolean
  selectedBackend: 'claude' | 'codex' | 'opencode' | 'cursor'
  selectedModel: string
  selectedProvider: string | null
  installedBackends: ('claude' | 'codex' | 'opencode' | 'cursor')[]
  customCliProfiles: CustomCliProfile[]
  sessionHasMessages?: boolean
  providerLocked?: boolean
  onModelChange: (model: string) => void
  onBackendModelChange: (
    backend: 'claude' | 'codex' | 'opencode' | 'cursor',
    model: string
  ) => void
  onRequestClose: () => void
  searchPlaceholder?: string
  className?: string
  commandListClassName?: string
}

export function BackendModelPickerContent({
  open,
  selectedBackend,
  selectedModel,
  selectedProvider,
  installedBackends,
  customCliProfiles,
  sessionHasMessages,
  providerLocked,
  onModelChange,
  onBackendModelChange,
  onRequestClose,
  searchPlaceholder = 'Search backends and models...',
  className,
  commandListClassName,
}: BackendModelPickerContentProps) {
  const [search, setSearch] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)

  const { data: availableOpencodeModels } = useAvailableOpencodeModels({
    enabled: installedBackends.includes('opencode'),
  })
  const { data: availableCursorModels } = useAvailableCursorModels({
    enabled: installedBackends.includes('cursor'),
  })

  const opencodeModelOptions = useMemo(
    () =>
      availableOpencodeModels?.map(model => ({
        value: model,
        label: formatOpencodeModelLabel(model),
      })),
    [availableOpencodeModels]
  )
  const cursorModelOptions = useMemo(
    () =>
      availableCursorModels?.map(model => ({
        value: `cursor/${model.id}`,
        label: model.label || formatCursorModelLabel(model.id),
      })),
    [availableCursorModels]
  )

  const { backendModelSections } = useToolbarDerivedState({
    selectedBackend,
    selectedProvider,
    selectedModel,
    opencodeModelOptions,
    cursorModelOptions,
    customCliProfiles,
    installedBackends,
  })

  const visibleSections = useMemo(() => {
    const allowedBackends = sessionHasMessages
      ? new Set([selectedBackend])
      : new Set(installedBackends)
    return backendModelSections.filter(section =>
      allowedBackends.has(section.backend)
    )
  }, [
    backendModelSections,
    installedBackends,
    selectedBackend,
    sessionHasMessages,
  ])

  const filteredSections = useMemo(() => {
    const query = search.trim().toLowerCase()
    return visibleSections
      .map(section => ({
        ...section,
        options: section.options.filter(
          option =>
            !query ||
            `${section.label} ${option.label} ${option.value}`
              .toLowerCase()
              .includes(query)
        ),
      }))
      .filter(section => section.options.length > 0)
  }, [search, visibleSections])

  useEffect(() => {
    if (!open) {
      setSearch('')
      return
    }

    const rafId = window.requestAnimationFrame(() => {
      searchInputRef.current?.focus()
      searchInputRef.current?.select()
    })

    return () => window.cancelAnimationFrame(rafId)
  }, [open])

  const handleSelect = useCallback(
    (backend: 'claude' | 'codex' | 'opencode' | 'cursor', model: string) => {
      if (backend === selectedBackend) {
        onModelChange(model)
      } else {
        onBackendModelChange(backend, model)
      }
      onRequestClose()
    },
    [onBackendModelChange, onModelChange, onRequestClose, selectedBackend]
  )

  const showProviderHint =
    Boolean(providerLocked) &&
    Boolean(sessionHasMessages) &&
    selectedBackend === 'claude' &&
    customCliProfiles.length > 0

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <Command shouldFilter={false} className="flex h-full flex-1 flex-col">
        <div className="border-b p-2">
          <Input
            ref={searchInputRef}
            value={search}
            spellCheck={false}
            onChange={event => setSearch(event.target.value)}
            onKeyDown={event => {
              if (event.key === 'Escape') {
                event.preventDefault()
                onRequestClose()
              }
            }}
            placeholder={searchPlaceholder}
            className="h-9 text-base md:text-sm"
          />
        </div>

        {showProviderHint && (
          <div className="px-4 pt-2 text-xs text-muted-foreground">
            Provider: {getProviderDisplayName(selectedProvider)}
          </div>
        )}

        <CommandList
          className={cn(
            'max-h-[24rem]',
            showProviderHint && 'pt-1',
            commandListClassName
          )}
        >
          {filteredSections.length === 0 && (
            <CommandEmpty>No models found.</CommandEmpty>
          )}

          {filteredSections.map(section => (
            <CommandGroup
              key={section.backend}
              heading={
                <BackendLabel
                  backend={section.backend}
                  badgeClassName="text-[9px] leading-3"
                />
              }
              className="[&_[cmdk-group-heading]]:sticky [&_[cmdk-group-heading]]:top-0 [&_[cmdk-group-heading]]:z-10 [&_[cmdk-group-heading]]:border-y [&_[cmdk-group-heading]]:bg-muted/95 [&_[cmdk-group-heading]]:backdrop-blur [&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:py-2 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-[0.18em] [&_[cmdk-group-heading]]:text-foreground/85"
            >
              {section.options.map(option => {
                const isSelected =
                  selectedBackend === section.backend &&
                  selectedModel === option.value

                return (
                  <CommandItem
                    key={`${section.backend}-${option.value}`}
                    value={`${getBackendPlainLabel(section.backend)} ${option.label} ${option.value}`}
                    onSelect={() => handleSelect(section.backend, option.value)}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="truncate">{option.label}</div>
                      <div className="truncate text-xs text-muted-foreground">
                        {option.value}
                      </div>
                    </div>
                    <Check
                      className={cn(
                        'ml-2 h-4 w-4 shrink-0',
                        isSelected ? 'opacity-100' : 'opacity-0'
                      )}
                    />
                  </CommandItem>
                )
              })}
            </CommandGroup>
          ))}
        </CommandList>
      </Command>
    </div>
  )
}
