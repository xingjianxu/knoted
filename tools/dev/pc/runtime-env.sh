# 手动 prepare 与自动部署共用。只读取开关、来源和凭据路径。
runtime_variable_keys=(
  KNOTED_SEARCH_ENABLED
  KNOTED_SEARCH_CREDENTIAL_FILE
  KNOTED_OFFICE_ORIGIN
  KNOTED_OFFICE_JWT_SECRET_FILE
)

load_deploy_env() {
  local file=$1 line key value number=0
  [[ -e $file || -L $file ]] || return 0
  if [[ ! -f $file || ! -r $file ]]; then
    printf '无法读取运行变量文件 %s\n' "$file" >&2
    return 1
  fi

  while IFS= read -r line || [[ -n $line ]]; do
    ((number += 1))
    [[ -z $line || $line == \#* ]] && continue
    if [[ ! $line =~ ^[A-Z_][A-Z_0-9]*= || $line == *$'\r'* ]]; then
      printf '%s:%d 需要 KEY=VALUE 格式\n' "$file" "$number" >&2
      return 1
    fi
    key=${line%%=*}
    value=${line#*=}
    case " ${runtime_variable_keys[*]} " in
    *" $key "*)
      if [[ -n $value && -z ${!key:-} ]]; then
        export "$key=$value"
      fi
      ;;
    esac
  done <"$file"
}
