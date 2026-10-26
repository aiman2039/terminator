FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server git neovim ca-certificates python3 && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --shell /bin/bash developer && passwd -d developer && mkdir -p /run/sshd /workspace/project /var/lib/ssh-host-keys && chown developer:developer /workspace/project
COPY sshd_config /etc/ssh/sshd_config.fixture
COPY entrypoint.sh /usr/local/bin/ssh-project-fixture
RUN chmod 755 /usr/local/bin/ssh-project-fixture
EXPOSE 22
ENTRYPOINT ["/usr/local/bin/ssh-project-fixture"]
