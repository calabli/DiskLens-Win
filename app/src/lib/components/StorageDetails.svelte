<script lang="ts">
  import { openStorageSettings, storageDetails, type StorageDetails } from "$lib/api";
  import { formatPercent, formatSize } from "$lib/format";

  interface Props {
    /** 数据版本；删除或刷新后递增，重新计算。 */
    version: number;
    onreveal: (id: number) => void;
    onclose: () => void;
  }

  let { version, onreveal, onclose }: Props = $props();

  /** 处理方式：可以清理 / 按步骤清理 / 不要删除。 */
  type Tone = "can" | "careful" | "keep";

  /** 各类系统位置的说明，`key` 与 Rust 侧 `storage::SYSTEM_ITEMS` 一致。 */
  const ABOUT: Record<string, { title: string; about: string; howto: string; tone: Tone }> = {
    pagefile: {
      title: "虚拟内存（页面文件）",
      about: "内存不够用时，Windows 把暂时用不到的数据放在这里；swapfile.sys 给应用商店的应用用。",
      howto:
        "由 Windows 自动管理，不要直接删除。确实需要时，在“高级系统设置 → 性能 → 高级 → 虚拟内存”里调小或移到其他磁盘。",
      tone: "keep",
    },
    hiberfil: {
      title: "休眠文件",
      about: "休眠和“快速启动”时把内存里的内容存在这里，大小约为内存的四成。",
      howto:
        "不用休眠和快速启动时，在管理员终端运行 powercfg /h off，文件会自动删除；只想保留快速启动，可以用 powercfg /h /type reduced 缩小。",
      tone: "careful",
    },
    restore: {
      title: "系统还原点与卷影副本",
      about: "系统保护创建的还原点、文件的“以前的版本”都存在这个隐藏文件夹里。",
      howto:
        "在“系统属性 → 系统保护 → 配置”里删除旧的还原点，或调低最多可用的空间。不要直接删除这个文件夹。",
      tone: "careful",
    },
    winsxs: {
      title: "Windows 组件存储（WinSxS）",
      about:
        "系统组件和更新的各个版本。其中很多文件和 System32 里的是同一份（硬链接），实际能释放的通常只是一小部分。",
      howto:
        "不要手动删除。在管理员终端运行 Dism /Online /Cleanup-Image /StartComponentCleanup，可以清掉已被替换的旧版本。",
      tone: "careful",
    },
    installer: {
      title: "Windows Installer 缓存",
      about: "已安装程序的安装包副本，修复、更新和卸载这些程序时要用到。",
      howto: "不要手动删除，否则相关程序可能无法更新或卸载。",
      tone: "keep",
    },
    update: {
      title: "Windows 更新缓存",
      about: "已下载的更新包，以及“传递优化”给其他电脑共享更新用的缓存。",
      howto:
        "更新装好以后可以清理：Windows 存储设置 → 临时文件，勾选“Windows 更新清理”和“传递优化文件”。",
      tone: "can",
    },
    windowsOld: {
      title: "旧版 Windows",
      about: "升级 Windows 后留下的旧系统，用来回退到之前的版本。",
      howto: "确定不再回退后，在 Windows 存储设置 → 临时文件里勾选“以前的 Windows 安装”删除。",
      tone: "can",
    },
    dumps: {
      title: "系统错误内存转储",
      about: "蓝屏或系统组件出错时保存的内存内容，用来排查问题。",
      howto: "不需要排查问题时可以删除：Windows 存储设置 → 临时文件 → 系统错误内存转储文件。",
      tone: "can",
    },
    temp: {
      title: "系统临时文件",
      about: "Windows 和安装程序运行时留下的临时文件。",
      howto: "在 Windows 存储设置 → 临时文件里清理，或使用 DiskLens 的清理建议。",
      tone: "can",
    },
    recycle: {
      title: "回收站",
      about: "删除到回收站的文件仍然占着这个磁盘的空间，所有用户的回收站都在这里。",
      howto: "确认不再需要后清空回收站。",
      tone: "can",
    },
    ntfs: {
      title: "NTFS 文件系统元数据",
      about: "文件系统自己的记录：主文件表 $MFT、日志等。文件越多，主文件表越大。",
      howto: "文件系统运行必需，不能删除。",
      tone: "keep",
    },
  };

  const TONE: Record<Tone, string> = {
    can: "可以清理",
    careful: "按说明清理",
    keep: "不要删除",
  };

  let details = $state<StorageDetails | null>(null);
  let error = $state("");

  $effect(() => {
    void version;
    storageDetails()
      .then((d) => {
        details = d;
        error = "";
      })
      .catch((e) => (error = String(e)));
  });

  const used = $derived(details?.volume ? details.volume.total - details.volume.free : 0);
  /** 整盘扫描时，已用空间里没被扫描结果覆盖的部分。 */
  const missing = $derived(
    details?.wholeVolume && details.volume ? Math.max(0, used - details.scanned) : 0,
  );
  const systemTotal = $derived((details?.items ?? []).reduce((sum, i) => sum + i.allocated, 0));

  async function settings() {
    try {
      await openStorageSettings();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<section class="storage" aria-label="存储详情">
  <header>
    <div class="title">
      <h2>存储详情</h2>
      <span
        >{details?.volume
          ? `${details.volume.label || "本地磁盘"} ${details.volume.root.slice(0, 2)} · ${details.volume.fileSystem}`
          : "这次扫描所在的磁盘"}</span
      >
    </div>
    <button type="button" class="secondary" onclick={settings}>打开 Windows 存储设置</button>
    <button type="button" class="close" onclick={onclose} aria-label="关闭">
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"
        ><path
          d="m4 4 8 8M12 4l-8 8"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        ></path></svg
      >
    </button>
  </header>

  {#if error}
    <p class="banner" role="alert">{error}</p>
  {/if}

  <div class="scroll">
    {#if details?.volume}
      {@const v = details.volume}
      {@const other = details.wholeVolume ? missing : Math.max(0, used - details.scanned)}
      <div class="usage">
        <div class="numbers">
          <strong>{formatSize(used)}</strong>
          <span>已用，共 {formatSize(v.total)}，可用 {formatSize(v.free)}</span>
        </div>
        <div class="bar" role="img" aria-label="磁盘用量">
          <span
            class="seg scanned"
            style:width="{(Math.min(details.scanned, used) / v.total) * 100}%"
          ></span>
          <span class="seg other" style:width="{(other / v.total) * 100}%"></span>
        </div>
        <div class="legend">
          <span
            ><i class="dot scanned"></i>{details.wholeVolume ? "扫描到的文件" : "这次扫描的文件夹"}
            {formatSize(details.scanned)}</span
          >
          {#if other > 0}
            <span
              ><i class="dot other"></i>{details.wholeVolume ? "没算进去的" : "磁盘上的其他内容"}
              {formatSize(other)}</span
            >
          {/if}
          <span><i class="dot free"></i>可用 {formatSize(v.free)}</span>
        </div>
        {#if details.wholeVolume && missing > v.total * 0.005}
          <p class="note">
            “没算进去的”是已用空间里扫描结果没有覆盖到的部分，通常是 Windows
            保留的存储空间、卷影副本和读不到的文件夹。以管理员身份扫描整个磁盘时，会改用 MFT
            直读，这部分会小很多。
          </p>
        {:else if details.wholeVolume && details.scanned > used * 1.002}
          <p class="note">
            {`扫描到的比磁盘报告的已用空间还多 ${formatSize(details.scanned - used)}：普通遍历时，很小的文件会多算一个簇，同一个文件的多个硬链接也会各算一次。以管理员身份扫描整个磁盘时会改用 MFT 直读，数字更准。`}
          </p>
        {:else if !details.wholeVolume}
          <p class="note">
            这次只扫描了一个文件夹。页面文件、还原点、WinSxS 这些系统占用要扫描整个磁盘（例如
            C:）才能列出来。
          </p>
        {/if}
      </div>
    {/if}

    {#if details?.wholeVolume}
      {#if details.items.length > 0}
        <h3>
          Windows 自己占用的空间 <span
            >{formatSize(systemTotal)}，占已用空间的 {formatPercent(systemTotal, used)}</span
          >
        </h3>
        {#each details.items as item (item.key)}
          {@const a = ABOUT[item.key]}
          <article class="card">
            <div class="head">
              <h4>{a?.title ?? item.key}</h4>
              <span class="tone {a?.tone ?? 'keep'}">{TONE[a?.tone ?? "keep"]}</span>
              <strong>{item.allocated > 0 ? formatSize(item.allocated) : "大小未知"}</strong>
            </div>
            {#if a}
              <p>{a.about}</p>
              <p class="howto"><b>怎样减小：</b>{a.howto}</p>
            {/if}
            <ul>
              {#each item.places as p (p.id)}
                <li>
                  <span class="path" title={p.path}>{p.path}</span>
                  <span class="size"
                    >{p.allocated > 0
                      ? formatSize(p.allocated)
                      : "读不到，以管理员身份运行才能看到"}</span
                  >
                  <button type="button" class="link" onclick={() => onreveal(p.id)}>定位</button>
                </li>
              {/each}
            </ul>
          </article>
        {/each}
      {:else}
        <p class="empty">这个磁盘上没有找到 Windows 的系统文件，它不是系统盘。</p>
      {/if}
    {/if}
  </div>
</section>

<style>
  .storage {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    border-radius: var(--radius);
    background: var(--panel);
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }

  .title {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: 15px;
  }

  .title span {
    color: var(--ink-2);
    font-size: 12px;
  }

  .secondary {
    height: 28px;
    padding: 0 14px;
    border: 0;
    border-radius: 7px;
    background: var(--field);
    color: var(--ink);
    font-weight: 500;
  }

  .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 50%;
    background: var(--field);
    color: var(--ink-2);
  }

  .banner {
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--warn-bg);
    color: var(--warn);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
  }

  .usage {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
    border-radius: 12px;
    background: var(--stripe);
  }

  .numbers {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .numbers strong {
    font-size: 26px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .numbers span {
    color: var(--ink-2);
  }

  .bar {
    display: flex;
    height: 14px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--track);
  }

  .seg.scanned,
  .dot.scanned {
    background: var(--accent);
  }

  .seg.other,
  .dot.other {
    background: repeating-linear-gradient(
      -45deg,
      var(--ink-3) 0 4px,
      color-mix(in srgb, var(--ink-3) 55%, transparent) 4px 8px
    );
  }

  .dot.free {
    background: var(--track);
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    color: var(--ink-2);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    margin-right: 6px;
    border-radius: 3px;
    vertical-align: -1px;
  }

  .note,
  .empty {
    margin: 0;
    color: var(--ink-2);
    font-size: 12px;
    line-height: 1.6;
  }

  .empty {
    margin-top: 16px;
  }

  h3 {
    margin: 22px 0 10px;
    font-size: 13px;
  }

  h3 span {
    margin-left: 6px;
    color: var(--ink-2);
    font-weight: 400;
  }

  .card {
    margin-bottom: 10px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--stripe);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  h4 {
    margin: 0;
    font-size: 13px;
  }

  .head strong {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }

  .tone {
    padding: 1px 7px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
  }

  .tone.can {
    background: var(--good-bg);
    color: var(--good);
  }

  .tone.careful {
    background: var(--warn-bg);
    color: var(--warn);
  }

  .tone.keep {
    background: var(--track);
    color: var(--ink-2);
  }

  .card p {
    margin: 6px 0 0;
    color: var(--ink-2);
    font-size: 12px;
    line-height: 1.6;
    user-select: text;
  }

  .card .howto {
    color: var(--ink);
  }

  ul {
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 3px 0;
    font-size: 12px;
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--ink-2);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    font-variant-numeric: tabular-nums;
  }

  .link {
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--accent);
  }

  .link:hover {
    background: var(--accent-soft);
  }
</style>
