// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import TitleBar from '~/components/TitleBar.vue'
import SettingsModal from '~/components/SettingsModal.vue'
import FileEditor from '~/components/FileEditor.vue'
import UsagesPopup from '~/components/UsagesPopup.vue'
import WiringPopover from '~/components/WiringPopover.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { openTab, resetTabs, switchTabs, tabId, useTabs } from '~/composables/useTabs'
import { useToasts } from '~/composables/useToasts'
import { closeUsages, jumpFrom, usagesPopup } from '~/composables/useSymbols'
import {
  classOfText,
  closeWiring,
  framework,
  frameworkToken,
  loadFramework,
  openWiring,
  wiring,
  type FrameworkInfo
} from '~/composables/useFramework'
import { lensText } from '~/composables/useCodeMirror'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))

const asked = vi.mocked(invoke)
const git = useGit()
const config = useConfig()
const tabs = useTabs()
const toasts = useToasts()

const MAGENTO: FrameworkInfo = { id: 'magento', name: 'Magento', version: '2.4.7', how: 'bin/magento', wiring: true }
const DI = 'app/code/BigBridge/Foo/etc/di.xml'
const DI_TEXT = '<config>\n    <type name="BigBridge\\Foo\\Model\\Thing"/>\n</config>\n'

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let answers: Record<string, unknown> = {}
let settings: Record<string, unknown> = {}
let wrappers: ReturnType<typeof mount>[] = []

function place(path: string, label: string | null = null) {
  return { path, line: 1, column: 1, kind: 'template', text: '<div/>', dependency: false, label }
}

beforeEach(async () => {
  calls = []
  answers = {}
  settings = { ai: {} }
  resetTabs()
  closeUsages()
  closeWiring()
  toasts.clear()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.inside = []
  git.store.refs = { locals: [], remotes: [], tags: [], stashes: [] } as never
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'config_get') return { version: 1, active_profile: null, global: settings, profiles: [] }
    if (cmd === 'config_set_global') {
      settings = (args as { global: Record<string, unknown> }).global
      return { version: 1, active_profile: null, global: settings, profiles: [] }
    }
    if (cmd === 'framework_info') return settings.frameworks === false ? null : MAGENTO
    if (cmd === 'read_file') {
      return { text: DI_TEXT, hash: 'h1', eol: 'lf', bom: false, readonly: null, missing: false }
    }
    return answers[cmd] ?? null
  })
  await config.load()
  await loadFramework('/repos/shop')
})

afterEach(() => {
  for (const one of wrappers) one.unmount()
  wrappers = []
  framework.info = null
})

const asks = (cmd: string) => calls.filter((one) => one.cmd === cmd)

describe('the framework chip', () => {
  it('sits by the project name and says what was detected', async () => {
    const bar = mount(TitleBar, {
      global: {
        components: { AppModal },
        stubs: { ProfileMenu: true, HistoryMenu: true, BranchDialog: true, Teleport: true }
      }
    })
    wrappers.push(bar)
    await flushPromises()
    const chip = bar.find('[data-testid="framework-chip"]')
    expect(chip.text()).toBe('Magento')
    expect(chip.attributes('title')).toBe('Magento 2.4.7 detected')

    framework.info = null
    await flushPromises()
    expect(bar.find('[data-testid="framework-chip"]').exists()).toBe(false)
  })
})

describe('⌘-click in a framework file', () => {
  it('opens the class an XML attribute names', async () => {
    answers.framework_links = {
      from: 18,
      to: 44,
      title: 'BigBridge\\Foo\\Model\\Thing',
      note: null,
      targets: [{ ...place('app/code/BigBridge/Foo/Model/Thing.php'), line: 5, kind: 'class' }],
      class: 'BigBridge\\Foo\\Model\\Thing'
    }
    openTab({ path: DI, mode: 'file' })
    const editor = mount(FileEditor, {
      attachTo: document.body,
      props: { tabId: tabId(DI), path: DI, hunks: [], runs: null, blame: false, find: null, fallback: null },
      global: { components: { AppModal } }
    })
    wrappers.push(editor)
    await flushPromises()
    const view = EditorView.findFromDOM(editor.find('.cm-editor').element as HTMLElement)!
    view.dispatch({ selection: { anchor: DI_TEXT.indexOf('Model') } })
    ;(editor.vm as unknown as { jumpAtCaret: () => void }).jumpAtCaret()
    await flushPromises()
    expect(asks('framework_links')[0]!.args).toMatchObject({ path: DI, offset: DI_TEXT.indexOf('Model') })
    // XML is no language the index reads: nothing else was asked.
    expect(asks('symbol_at')).toHaveLength(0)
    const active = tabs.state.tabs.find((one) => one.id === tabs.state.active)
    expect(active?.path).toBe('app/code/BigBridge/Foo/Model/Thing.php')
  })

  it('lists the candidates of a template, the module’s own first, with their labels', async () => {
    answers.framework_links = {
      from: 0,
      to: 10,
      title: 'Template BigBridge_Foo::info.phtml · 2',
      note: null,
      targets: [
        place('app/code/BigBridge/Foo/view/frontend/templates/info.phtml', 'frontend · module'),
        place(
          'app/design/frontend/BigBridge/default/BigBridge_Foo/templates/info.phtml',
          'frontend · BigBridge/default · override'
        )
      ],
      class: null
    }
    await jumpFrom('app/code/BigBridge/Foo/Block/Info.php', 40, "<?php $t = 'BigBridge_Foo::info.phtml';", {
      x: 10,
      y: 10
    })
    expect(usagesPopup.open).toBe(true)
    const popup = mount(UsagesPopup, { attachTo: document.body })
    wrappers.push(popup)
    await flushPromises()
    expect(popup.find('.title').text()).toBe('Template BigBridge_Foo::info.phtml · 2')
    const rows = popup.findAll('.row')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.find('.tag').text()).toBe('frontend · module')
    expect(rows[1]!.find('.tag').text()).toBe('frontend · BigBridge/default · override')
    expect(rows[1]!.find('.tag').classes()).toContain('override')
    await rows[1]!.trigger('click')
    const active = tabs.state.tabs.find((one) => one.id === tabs.state.active)
    expect(active?.path).toBe('app/design/frontend/BigBridge/default/BigBridge_Foo/templates/info.phtml')
  })

  it('says a generated class was opened through its source', async () => {
    answers.framework_links = {
      from: 0,
      to: 5,
      title: 'X',
      note: 'OrderFactory is generated (factory); opened Order',
      targets: [{ ...place('vendor/magento/module-sales/Model/Order.php'), kind: 'class', dependency: true }],
      class: 'Magento\\Sales\\Model\\Order'
    }
    await jumpFrom(DI, 3, DI_TEXT)
    expect(toasts.items.value.map((one) => one.title)).toContain('OrderFactory is generated (factory); opened Order')
  })

  it('leaves the click to the language when the framework has nothing there', async () => {
    answers.framework_links = null
    answers.symbol_at = null
    await jumpFrom('app/code/BigBridge/Foo/Block/Info.php', 3, '<?php $x = 1;')
    expect(asks('symbol_at')).toHaveLength(1)
  })

  it('underlines the whole class name or template string under ⌘', () => {
    const line = '<type name="BigBridge\\Foo\\Model\\Thing"/>'
    const found = frameworkToken(line, line.indexOf('Model'))!
    expect(line.slice(found.from, found.to)).toBe('BigBridge\\Foo\\Model\\Thing')
    const php = "$_template = 'BigBridge_Foo::catalog/options.phtml';"
    const template = frameworkToken(php, php.indexOf('catalog'))!
    expect(php.slice(template.from, template.to)).toBe('BigBridge_Foo::catalog/options.phtml')
    expect(frameworkToken('$plain = word;', 3)).toBeNull()
  })
})

describe('config references in the usage counts', () => {
  it('are said after the usages, or on their own', () => {
    expect(lensText({ name: 'A', kind: 'class', line: 1, from: 0, count: 3, config: 2 }).main).toBe(
      '3 usages · 2 in config'
    )
    expect(lensText({ name: 'A', kind: 'class', line: 1, from: 0, count: 0, config: 1 }).main).toBe(
      '1 reference in config'
    )
  })

  it('are grouped under the framework’s heading in the popup', async () => {
    answers.usages = {
      definite: [],
      possible: [],
      framework: false,
      owner: null,
      config: [{ path: DI, line: 4, column: 9, kind: 'config', text: '<plugin type="…"/>' }],
      config_label: 'Magento config'
    }
    const { showUsages } = await import('~/composables/useSymbols')
    await showUsages('ProductPlugin', 'app/code/BigBridge/Foo/Plugin/ProductPlugin.php', 'class', { x: 1, y: 1 })
    const popup = mount(UsagesPopup, { attachTo: document.body })
    wrappers.push(popup)
    await flushPromises()
    expect(popup.find('[data-testid="config-heading"]').text()).toBe('Magento config (1)')
    expect(popup.find('.row .num').text()).toBe(':4')
  })
})

describe('the wiring view', () => {
  it('shows what magequery said, each row opening its class or its config', async () => {
    answers.framework_wiring = {
      class: 'Magento\\Catalog\\Model\\Product',
      kind: 'class',
      module: 'Magento_Catalog',
      path: 'vendor/magento/module-catalog/Model/Product.php',
      preference: 'BigBridge\\Foo\\Model\\Product',
      sections: [
        {
          title: 'Plugins',
          rows: [
            {
              label: 'bb_name',
              detail: 'BigBridge\\Foo\\Plugin\\ProductPlugin · sort 10 · global · after getName',
              class: 'BigBridge\\Foo\\Plugin\\ProductPlugin',
              path: DI,
              line: 4
            }
          ]
        },
        { title: 'Web API', rows: [{ label: '/V1/products', detail: 'GET', class: null, path: null, line: null }] }
      ]
    }
    answers.framework_class = { ...place('app/code/BigBridge/Foo/Plugin/ProductPlugin.php'), line: 5, kind: 'class' }
    await openWiring('Magento\\Catalog\\Model\\Product')
    expect(asks('framework_wiring')[0]!.args).toMatchObject({ class: 'Magento\\Catalog\\Model\\Product' })
    const view = mount(WiringPopover, { attachTo: document.body })
    wrappers.push(view)
    await flushPromises()
    expect(view.find('[data-testid="wiring-preference"]').text()).toContain('BigBridge\\Foo\\Model\\Product')
    const titles = view.findAll('.section h3').map((one) => one.text())
    expect(titles).toEqual(['Plugins 1', 'Web API 1'])
    expect(view.find('.row .detail').text()).toContain('sort 10')
    expect(view.find('.row .source').text()).toBe('di.xml:4')
    // A row with no file in the work tree has nothing to open.
    expect(view.findAll('.row')[1]!.find('.source').exists()).toBe(false)
    await view.find('.row .label.link').trigger('click')
    await flushPromises()
    expect(asks('framework_class')[0]!.args).toMatchObject({ class: 'BigBridge\\Foo\\Plugin\\ProductPlugin' })
    expect(wiring.open).toBe(false)
    const active = tabs.state.tabs.find((one) => one.id === tabs.state.active)
    expect(active?.path).toBe('app/code/BigBridge/Foo/Plugin/ProductPlugin.php')
  })

  it('says plainly when magequery could not run', async () => {
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'framework_wiring') throw 'magequery took longer than 10 seconds and was stopped'
      return null
    })
    await openWiring('Foo\\Bar')
    const view = mount(WiringPopover, { attachTo: document.body })
    wrappers.push(view)
    await flushPromises()
    expect(view.find('[data-testid="wiring-error"]').text()).toBe(
      'magequery took longer than 10 seconds and was stopped'
    )
  })

  it('reads the class a PHP file declares', () => {
    expect(classOfText('<?php\nnamespace A\\B;\n\nfinal class C extends D {}')).toBe('A\\B\\C')
    expect(classOfText('<?php\ninterface Only {}')).toBe('Only')
    expect(classOfText('<?php\nreturn [];')).toBeNull()
  })
})

describe('the Framework support setting', () => {
  it('turns detection and framework links off', async () => {
    config.store.settingsSection = 'frameworks'
    answers.magequery_status = { path: '/Applications/gitnoob.app/Contents/MacOS/magequery', source: 'bundled', pinned: '0.24.1', missing_setting: false }
    const modal = mount(SettingsModal, { global: { components: { AppModal } } })
    wrappers.push(modal)
    await flushPromises()
    expect(modal.find('[data-testid="magequery-status"]').text()).toContain('bundled, 0.24.1')
    expect(modal.find('[data-testid="magequery-notice"]').text()).toContain('EUPL-1.2')
    const box = modal.find('[data-setting="frameworks"]')
    expect((box.element as HTMLInputElement).checked).toBe(true)
    ;(box.element as HTMLInputElement).checked = false
    await box.trigger('change')
    await flushPromises()
    expect(asks('config_set_global').at(-1)!.args).toMatchObject({ global: { frameworks: false } })

    await loadFramework('/repos/shop')
    expect(framework.info).toBeNull()
    calls = []
    answers.symbol_at = null
    await jumpFrom(DI, 10, DI_TEXT)
    expect(asks('framework_links')).toHaveLength(0)
  })
})
