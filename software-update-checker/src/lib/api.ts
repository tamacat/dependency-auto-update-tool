// Rust コマンドの薄いラッパー。invoke の名前と引数はここにだけ書く。

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  CheckOptions,
  CheckProgress,
  CheckResult,
  Component,
  DataSource,
  ImportResult,
  JavaRequirement,
  JavaRule,
  KnowledgeView,
  LogEntry,
  MergeSummary,
  RuleKey,
} from './types';

export const importFile = (path: string) => invoke<ImportResult>('import_file', { path });

export const generateSbomWithMaven = (pomPath: string, options: CheckOptions) =>
  invoke<ImportResult>('generate_sbom_with_maven', { pomPath, options });

export const runChecks = (components: Component[], options: CheckOptions) =>
  invoke<CheckResult[]>('run_checks', { components, options });

export const javaRequirements = (component: Component, versions: string[], options: CheckOptions) =>
  invoke<JavaRequirement[]>('java_requirements', { component, versions, options });

export const knowledgeList = (sharedFiles: string[]) => invoke<KnowledgeView>('knowledge_list', { sharedFiles });

export const knowledgeUpsert = (rule: JavaRule, original: RuleKey | null) =>
  invoke<void>('knowledge_upsert', { rule, original });

export const knowledgeDelete = (key: RuleKey) => invoke<boolean>('knowledge_delete', { key });

export const knowledgeImport = (path: string) => invoke<MergeSummary>('knowledge_import', { path });

/** 保存先はバックエンドのダイアログで選ばせる。キャンセルなら null。 */
export const knowledgeExport = (includeDetected: boolean) =>
  invoke<{ path: string; count: number } | null>('knowledge_export', { includeDetected });

export const setLanguage = (language: string) => invoke<void>('set_language', { language });

export const dataSources = () => invoke<DataSource[]>('data_sources');

export const activityLog = () => invoke<LogEntry[]>('activity_log');

export const logDirectory = () => invoke<string | null>('log_directory');

/** 保存先はバックエンドのダイアログで選ばせる。キャンセルなら null。 */
export const exportReport = (kind: 'csv' | 'json', defaultName: string, contents: string) =>
  invoke<string | null>('export_report', { kind, defaultName, contents });

export const clearCache = () => invoke<void>('clear_cache');

export const onCheckProgress = (handler: (p: CheckProgress) => void): Promise<UnlistenFn> =>
  listen<CheckProgress>('check-progress', (e) => handler(e.payload));

export const onActivityLog = (handler: (e: LogEntry) => void): Promise<UnlistenFn> =>
  listen<LogEntry>('activity-log', (e) => handler(e.payload));
