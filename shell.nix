# https://github.com/RKlompUU/nix-recipes/blob/main/pg-server/shell.nix
{ target ? "default", pkgs ? import <nixpkgs> {} }:
with pkgs;

mkShell {
  name = "postgres-env";
  buildInputs = [
    postgresql_15
    less
    glibcLocales
    elixir
  ];

  postgresConf =
    writeText "postgresql.conf"
      ''
        log_min_messages = warning
        log_min_error_statement = error
        log_min_duration_statement = 100  # ms
        log_connections = on
        log_disconnections = on
        log_duration = on
        log_timezone = 'UTC'
        log_statement = 'all'
        log_directory = 'logs'
        log_filename = 'postgresql-%Y-%m-%d_%H%M%S.log'
        logging_collector = on
        log_min_error_statement = error
      '';


  PGDATA = "${toString ./.}/" + target + "/.pg";
  #PGUSER = "devuser";
  #PGPORT = 47923;

  shellHook = "mkdir -p " + target + "; " + ''
    echo "Using ${postgresql_15.name}."

    set -a
    source ./env
    set +a

    [ ! -d $PGDATA ] && PGHOST="$PGDATA" pg_ctl initdb -o "-U treadmill_dev" && cat "$postgresConf" >> $PGDATA/postgresql.conf

    pg_ctl -o "-k $PGDATA" start && {
      trap 'pg_ctl stop' EXIT
    }
  '';
}
