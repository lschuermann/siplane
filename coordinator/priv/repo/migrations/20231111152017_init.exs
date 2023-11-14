defmodule Siplane.Repo.Migrations.Init do
  use Ecto.Migration

  def change do
    # Log events relate to different entities, which we keep in individual
    # relations. These relations also define whether a given log entry should be
    # visible to those entities.
    create table(:log_events, primary_key: false) do
      add :id, :binary_id, primary_key: true
      add :severity, :int, null: false
      add :event_type, :string, null: false
      add :event_version, :string, null: false
      add :message, :string, null: false
      add :data, :map

      timestamps(type: :utc_datetime)
    end

    create table(:users, primary_key: false) do
      add :id, :binary_id, primary_key: true, null: false
      add :name, :string, null: false
      add :email, :string, null: false

      # User-profile configuration options
      add :auto_import_gh_ssh_keys, :bool, default: true, null: false

      timestamps(type: :utc_datetime)
    end

    create unique_index(:users, [:email])

    create table(:log_event_users, primary_key: false) do
      add :log_event_id, references(:log_events, type: :binary_id), primary_key: true
      add :user_id, references(:users, type: :binary_id), primary_key: true
      add :user_visible, :bool, null: false
    end

    create table(:user_ssh_keys, primary_key: false) do
      add :id, :binary_id, primary_key: true, null: false
      add :user_id, references(:users, type: :binary_id), null: false

      add :enabled, :bool, null: false, default: true
      add :type, :string, null: false
      add :binary_key, :binary, null: false
      add :label, :string

      # References to import sources
      add :github_ssh_key_id, :int

      timestamps(type: :utc_datetime)
    end

    create unique_index(:user_ssh_keys, [:type, :binary_key])

    create table(:user_providers, primary_key: false) do
      add :user_id, references(:users, type: :binary_id), primary_key: true, null: false
      add :provider, :string, primary_key: true, null: false
      add :token, :string, null: false

      timestamps(type: :utc_datetime)
    end

    create table(:boards, primary_key: false) do
      add :id, :binary_id, primary_key: true, null: false
      add :label, :string, null: false
      add :manufacturer, :string, null: false
      add :model, :string, null: false
      add :image_url, :string, null: true
      add :hwrev, :string, null: true
      add :location, :string, null: false
      add :runner_token, :string, null: false

      timestamps(type: :utc_datetime)
    end

    create table(:log_event_boards, primary_key: false) do
      add :log_event_id, references(:log_events, type: :binary_id), primary_key: true
      add :board_id, references(:boards, type: :binary_id), primary_key: true

      add :public, :bool, null: false
      # TODO: this should capture the owners at the time that this was logged,
      # and individually be able to control whether it is visible for just the
      # current owners, past owners, or both.
      add :owner_visible, :bool, null: false
    end

    create table(:board_owners, primary_key: false) do
      add :board_id, references(:boards, type: :binary_id), primary_key: true
      add :user_id, references(:users, type: :binary_id), primary_key: true

      timestamps(type: :utc_datetime)
    end

  end
end
