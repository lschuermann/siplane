defmodule Treadmill.Repo.Migrations.AddJobParameters do
  use Ecto.Migration

  def change do
    create table(:job_parameters, primary_key: false) do
      add :job_id, references(:jobs, type: :binary_id), null: false, primary_key: true
      add :key, :string, null: false, size: 64, primary_key: true
      add :value, :string, null: false, size: (64 * 1024)
      add :secret, :bool, null: false

      timestamps(type: :utc_datetime)
    end

    create table(:environment_parameters, primary_key: false) do
      add :environment_id, references(:environments, type: :binary_id), null: false, primary_key: true
      add :key, :string, null: false, size: 64, primary_key: true
      add :value, :string, null: false, size: (64 * 1024)
      add :secret, :bool, null: false

      timestamps(type: :utc_datetime)
    end

    create table(:board_parameters, primary_key: false) do
      add :board_id, references(:boards, type: :binary_id), null: false, primary_key: true
      add :key, :string, null: false, size: 64, primary_key: true
      add :value, :string, null: false, size: (64 * 1024)
      add :secret, :bool, null: false

      timestamps(type: :utc_datetime)
    end

    create table(:board_environment_parameters, primary_key: false) do
      # This uses a composite primary key of {:environment_id, :board_id, :key},
      # of which {:environment_id, :board_id} is a foreign key relation with the
      # board_environments table.
      add :environment_id, :binary_id, null: false, primary_key: true
      add :board_id, references(
	:board_environments,
	name: "board_environment_parameters_board_environment_id_fkey",
	column: :board_id,
	with: [environment_id: :environment_id],
	type: :binary_id
      ), null: false, primary_key: true

      add :key, :string, null: false, size: 64, primary_key: true
      add :value, :string, null: false, size: (64 * 1024)
      add :secret, :bool, null: false

      timestamps(type: :utc_datetime)
    end
  end
end
