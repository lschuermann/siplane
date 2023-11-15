defmodule Siplane.Log do
  use Ecto.Schema
  # import Ecto.Changeset

  # TODO: these constants should be automatically inserted into
  # log-messages using the appropriate methods:
  def severity_critical, do: 1
  def severity_error, do: 2
  def severity_warn, do: 3
  def severity_info, do: 4
  def severity_debug, do: 5
  def severity_trace, do: 6

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "log_events" do
    field :severity, :integer
    field :event_type, :string
    field :event_version, :string
    field :message, :string
    field :data, :map

    timestamps(type: :utc_datetime)

    has_many :log_event_users, Siplane.User.LogEvent, foreign_key: :log_event_id
    has_many :log_event_boards, Siplane.Board.LogEvent, foreign_key: :log_event_id
  end

  def info(record) do
    inserted =
      Siplane.Repo.insert!(record)
      |> Siplane.Repo.preload([:log_event_users, :log_event_boards])

    if !(Enum.empty? inserted.log_event_users) do
      Siplane.User.log_event(inserted)
    end

    if !(Enum.empty? inserted.log_event_boards) do
      Siplane.Board.log_event(inserted)
    end
  end
end

