import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import test from 'node:test'

const read = path => readFileSync(path, 'utf8')

test('server release workflow builds binaries and publishes docker image', () => {
  const workflow = read('.github/workflows/server-release.yml')

  assert.match(workflow, /cargo build --locked --release -p jean-server/)
  assert.match(workflow, /cargo tree -p jean-server/)
  assert.match(workflow, /readelf -d target\/release\/jean-server/)
  assert.match(workflow, /ldd target\/release\/jean-server/)
  assert.match(workflow, /jean-server-linux-amd64/)
  assert.match(workflow, /jean-server-linux-arm64/)
  assert.match(workflow, /docker\/build-push-action@v6/)
  assert.match(workflow, /ghcr\.io/)
  assert.doesNotMatch(workflow, /libwebkit2gtk-4\.1-dev/)
})

test('Dockerfile builds and runs jean-server headlessly as non-root user', () => {
  const dockerfile = read('Dockerfile.server')

  assert.match(dockerfile, /bun run build/)
  assert.match(dockerfile, /COPY crates \.\/crates/)
  assert.match(dockerfile, /COPY src-server \.\/src-server/)
  assert.match(
    dockerfile,
    /cargo build --locked --release -p jean-server/
  )
  assert.match(dockerfile, /USER jean/)
  assert.match(dockerfile, /chown -R jean:jean \/home\/jean/)
  assert.match(dockerfile, /JEAN_HOST=0\.0\.0\.0/)
  assert.doesNotMatch(dockerfile, /xvfb|webkit|gtk/i)
  assert.match(dockerfile, /ENTRYPOINT \["\/usr\/local\/bin\/jean-server"\]/)
})

test('jean-server depends on jean-core and never on the desktop Tauri package', () => {
  const workspaceCargoToml = read('Cargo.toml')
  const cargoToml = read('src-tauri/Cargo.toml')
  const serverCargoToml = read('src-server/Cargo.toml')
  const coreCargoToml = read('crates/jean-core/Cargo.toml')

  assert.match(workspaceCargoToml, /"crates\/jean-core"/)
  assert.match(workspaceCargoToml, /"src-server"/)
  assert.doesNotMatch(cargoToml, /jean-server/)
  assert.match(serverCargoToml, /name = "jean-server"/)
  assert.match(serverCargoToml, /jean-core = \{ path = "\.\.\/crates\/jean-core" \}/)
  assert.doesNotMatch(serverCargoToml, /tauri|jean_lib/)
  assert.doesNotMatch(coreCargoToml, /tauri|gtk|webkit|wry/)
})

test('Docker entrypoint starts jean-server without a virtual display', () => {
  const entrypoint = read('scripts/docker-entrypoint.sh')

  assert.doesNotMatch(entrypoint, /Xvfb|DISPLAY|WAYLAND_DISPLAY/)
  assert.match(entrypoint, /exec jean-server "\$@"/)
})

test('remote provisioning prefers the checksummed GUI-free server artifact', () => {
  const provision = read('src-tauri/src/remote/provision.rs')

  assert.match(provision, /jean-server-linux-\{arch\}/)
  assert.match(provision, /Sha256::digest/)
  assert.match(
    provision,
    /\/opt\/jean-remote\/jean-server --host 127\.0\.0\.1/
  )
  assert.match(provision, /NoNewPrivileges=true/)
  assert.match(provision, /ProtectSystem=full/)
  const primaryDependencies = provision.match(
    /fn dependency_install_command\(\)[\s\S]*?\n}\n\nfn compatibility_dependency_install_command/
  )?.[0]
  assert.ok(primaryDependencies)
  assert.doesNotMatch(primaryDependencies, /xvfb|webkit|gtk/i)
})

test('desktop and server share jean-core git business logic', () => {
  const commands = read('src-tauri/src/projects/commands.rs')
  const desktopGit = read('src-tauri/src/projects/git.rs')
  const runtime = read('src-tauri/src/backend_runtime.rs')

  assert.match(commands, /backend_runtime::git_service\(\)/)
  assert.match(runtime, /GitService::new\(desktop_git_runner\)/)
  assert.match(runtime, /wsl_aware_command\("git"/)
  assert.doesNotMatch(
    desktopGit,
    /pub fn (get_git_remotes|get_branches|git_pull|git_stash|git_stash_pop|git_push|commit_changes|has_uncommitted_changes)\(/
  )
  assert.match(desktopGit, /git_service\(\)\.branch_exists/)
  assert.match(desktopGit, /git_service\(\)[\s\S]*?\.valid_base_branch/)
  assert.match(desktopGit, /git_service\(\)[\s\S]*?\.create_worktree\(/)
  assert.match(
    desktopGit,
    /git_service\(\)[\s\S]*?\.create_worktree_from_existing_branch\(/
  )
  assert.doesNotMatch(desktopGit, /fn run_git_worktree_add_with_retry/)
  assert.doesNotMatch(desktopGit, /fn remove_dir_all_with_retry/)
  assert.doesNotMatch(desktopGit, /pub fn is_main_worktree/)
  assert.match(desktopGit, /git_service\(\)[\s\S]*?\.remove_worktree\(/)
  assert.match(desktopGit, /git_service\(\)[\s\S]*?\.delete_branch\(/)
  assert.equal(existsSync('src-tauri/src/projects/git_log.rs'), false)
})

test('desktop project commands delegate simple business operations to jean-core', () => {
  const commands = read('src-tauri/src/projects/commands.rs')
  const runtime = read('src-tauri/src/backend_runtime.rs')

  assert.match(runtime, /ProjectService::with_services\(/)
  assert.match(runtime, /context\.persistence,[\s\S]*git_service\(\),[\s\S]*script_service\(\)/)
  for (const command of [
    'init_project',
    'clone_project',
    'get_worktree_changes',
    'get_worktree_diff',
    'create_base_session',
    'archive_worktree',
    'unarchive_worktree',
    'list_archived_worktrees',
    'import_worktree',
    'permanently_delete_worktree',
    'create_worktree_from_existing_branch',
    'rename_worktree',
    'update_worktree_labels',
    'set_worktree_last_opened',
    'update_worktree_cached_status',
    'get_project_branches',
    'update_project_settings',
    'reorder_projects',
    'reorder_worktrees',
  ]) {
    const start = commands.indexOf(`pub async fn ${command}`)
    assert.notEqual(start, -1, `missing ${command}`)
    const nextCommand = commands.indexOf('#[tauri::command]', start + 1)
    const body = commands.slice(start, nextCommand === -1 ? undefined : nextCommand)
    assert.match(body, /backend_runtime::project_service\(&app\)/, command)
  }

  const desktopStatus = read('src-tauri/src/projects/git_status.rs')
  assert.match(desktopStatus, /backend_runtime::git_service\(\)/)
  assert.doesNotMatch(desktopStatus, /Command::new|wsl_aware_command/)
  const closeBase = commands.slice(
    commands.indexOf('async fn close_base_session_internal'),
    commands.indexOf('// Archive Commands')
  )
  assert.match(closeBase, /backend_runtime::project_service\(app\)/)
  assert.match(closeBase, /BaseSessionCloseMode/)
  assert.doesNotMatch(closeBase, /preserve_base_sessions|with_sessions_mut|delete_session_data/)
  const existingBranch = commands.slice(
    commands.indexOf('pub async fn create_worktree_from_existing_branch'),
    commands.indexOf('pub async fn checkout_pr')
  )
  assert.match(existingBranch, /prepare_existing_branch_worktree/)
  assert.match(existingBranch, /run_existing_branch_worktree_task/)
  assert.doesNotMatch(existingBranch, /git::create_worktree|format_issue_context_markdown|save_projects_data/)
})
