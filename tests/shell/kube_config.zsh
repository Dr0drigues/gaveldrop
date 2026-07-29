kube_config_show() {
    ui_path "$ZSH_ENV_DIR/scripts/fmt.zsh"
}

kube_config_write() {
    mkdir -p "$HOME/.kube"
    printf 'current-context: %s\n' "$1" > "$HOME/.kube/config"
}
