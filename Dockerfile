# syntax=docker/dockerfile:1.7

# ===== Builder 阶段 =====
FROM rust:1.85-slim AS builder
WORKDIR /build

# 安装构建依赖 (rusqlite bundled 需要 cc,tree-sitter grammar 需要 cc)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# 先复制 workspace 依赖描述文件,利用 layer 缓存
COPY Cargo.toml Cargo.lock ./
COPY crates/devnpc-core/Cargo.toml ./crates/devnpc-core/
COPY crates/devnpc/Cargo.toml ./crates/devnpc/

# 创建空源文件占位,让 cargo 能解析依赖图并预编译依赖 (利用 cache mount)
RUN mkdir -p crates/devnpc-core/src crates/devnpc/src && \
    echo "" > crates/devnpc-core/src/lib.rs && \
    echo "" > crates/devnpc/src/lib.rs && \
    echo "" > crates/devnpc/src/main.rs

# 用 BuildKit cache mount 预编译依赖 (失败不中断,实际源码复制后会重新编译)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release || true

# 复制实际源码 (workspace 拆分后源码位于 crates/devnpc/src/ 和 crates/devnpc-core/src/)
COPY crates/devnpc-core/src/ ./crates/devnpc-core/src/
COPY crates/devnpc/src/ ./crates/devnpc/src/
COPY crates/devnpc/tests/ ./crates/devnpc/tests/

# 构建二进制并复制到 /usr/local/bin
# 注意: target 目录在 cache 中,构建完需 cp 出二进制
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release && \
    cp target/release/devnpc /usr/local/bin/devnpc

# ===== Runtime 阶段 =====
FROM debian:bookworm-slim AS runtime

# 安装运行时最小依赖:
# - git: GitOps 工具调用 (git/ops.rs)
# - ca-certificates: HTTPS 调用 GitLab API
# - tini: 作为 PID 1 处理信号转发 (webhook serve 模式优雅关闭)
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/false devnpc

COPY --from=builder /usr/local/bin/devnpc /usr/local/bin/devnpc

# 非 root 用户运行 (安全加固)
USER devnpc
WORKDIR /workspace

# tini 作为 init 进程,确保信号正确转发
ENTRYPOINT ["tini", "--", "devnpc"]
CMD ["--help"]

LABEL org.opencontainers.image.title="devnpc" \
      org.opencontainers.image.description="基于 GitLab 的企业级研发流程 AI 智能体" \
      org.opencontainers.image.source="https://github.com/duantianjun/devnpc" \
      org.opencontainers.image.licenses="MIT"
