# Shared by dev startup and deployment. Persist settings and paths, never credential contents.
runtime_variable_keys=(
  KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR
  KNOTED_INTELLIGENCE_BGE_M3_DIR
  KNOTED_SEARCH_ENABLED
  KNOTED_SEARCH_CREDENTIAL_FILE
  KNOTED_VECTORS_ENABLED
  KNOTED_OFFICE_ORIGIN
  KNOTED_OFFICE_JWT_SECRET_FILE
  KNOTED_CHAT_LOCAL_BASE_URL
  KNOTED_CHAT_LOCAL_MODEL
  KNOTED_CHAT_LOCAL_CREDENTIAL_FILE
)

load_deploy_env() {
  local file=$1 line key value lineno=0
  if [[ ! -e $file && ! -L $file ]]; then
    return 0
  fi
  if [[ ! -f $file || ! -r $file ]]; then
    printf '无法读取运行变量文件 %s\n' "$file" >&2
    return 1
  fi
  while IFS= read -r line || [[ -n $line ]]; do
    ((lineno += 1))
    [[ -z $line || $line == \#* ]] && continue
    if [[ ! $line =~ ^[A-Z_][A-Z_0-9]*= || $line == *$'\r'* ]]; then
      printf '%s:%d 必须是 KEY=VALUE，不能执行 Shell 语法\n' "$file" "$lineno" >&2
      return 1
    fi
    key=${line%%=*}
    value=${line#*=}
    case " ${runtime_variable_keys[*]} " in
    *" $key "*)
      if [[ -n $value && -z ${!key:-} ]]; then export "$key=$value"; fi
      ;;
    esac
  done <"$file"
}
