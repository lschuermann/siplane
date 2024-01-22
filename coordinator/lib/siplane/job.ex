defmodule Siplane.Job do
  use Supervisor
  use Ecto.Schema
  import Ecto.Changeset
  import Ecto.Query

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "jobs" do
    field :label, :string

    field :start, :utc_datetime
    field :end, :utc_datetime
    field :dispatched, :boolean
    field :completion_code, Ecto.Enum, values: [
      # Never ran, user-cancelled:
      :cancelled,
      # The board server crashed while this job was active:
      :server_crashed,
      # We encountered a timeout while talking to the runner for this job:
      :runner_timeout,
      # Finished sucessfully prior to the job's end-time:
      :finished,
      # Job finished prior to the job's end-time, but failed:
      :failed,
      # Job was aborted because it reached its end-time:
      :expired,
      # Job was forcefully aborted by a user:
      :aborted,
    ]

    timestamps(type: :utc_datetime)

    belongs_to :board, Siplane.Board
    belongs_to :environment, Siplane.Board
    belongs_to :creator, Siplane.User
  end

  @doc false
  def changeset(user, attrs) do
    # TODO: board & environment foreign key constraint?
    user
    |> cast(attrs, [:label, :start, :end, :dispatched, :completion_code])
    |> validate_required([:start, :dispatched])
  end

  # ----- Public-Facing API ----------------------------------------------------

  def validate_job_id(job_id) do
    # Validate that this job_id is a valid binary UUID:
    if !is_binary(job_id) || byte_size(job_id) != 16 do
      raise ArgumentError, message: "invalid argument job_id: #{inspect job_id}"
    end
  end


  # Supervise a registry of board servers assigned to a given job, and
  # a registry of subscribers to board events. We can't combine them
  # in one registry, as we want multiple subscribers for a job, but
  # never multiple board servers for a job.
  def start_link(init_arg) do
    Supervisor.start_link(__MODULE__, init_arg, name: __MODULE__)
  end

  # TODO: document. This is a callback, right?
  def log_event(_event) do
    # nada
  end

  # Queries the next `limit` pending jobs, optionally filtered for a
  # given board. Excludes jobs currently scheduled on a board. Jobs
  # which have not been completed but whose end time has already
  # expired are not included.
  def pending_jobs(opts) do
    now = DateTime.utc_now

    query = from j in Siplane.Job,
      where: is_nil(j.completion_code) and (is_nil(j.end) or j.end > ^now) and j.dispatched == false,
      order_by: [asc: j.start]

    query =
      case Keyword.get(opts, :board_id) do
	nil -> query
	board_id ->
          Siplane.Board.validate_board_id(board_id)
	  ecto_board_id = Ecto.UUID.load! board_id
	  where(query, board_id: ^ecto_board_id)
      end

    query =
      case Keyword.get(opts, :limit) do
	nil -> query
	limit_val -> limit(query, ^limit_val)
      end

    Siplane.Repo.all(query)
    |> Enum.filter(fn job ->
      # Don't include jobs which are currently scheduled on a runner:
      !job_active?(UUID.string_to_binary!(job.id))
    end)
  end

  def active_jobs(opts) do
    ecto_job_ids =
      case Keyword.get(opts, :board_id) do
	nil ->
	  # Magic to get all keys from the Registry:
	  Registry.select(__MODULE__.BoardRegistry, [{{:"$1", :_, :_}, [], [:"$1"]}])
	board_id ->
	  Siplane.Board.validate_board_id(board_id)
	  # Look up the PID of the board server for the given board_id,
	  # if we have one.
	  case Registry.lookup(Siplane.Board.Server.Registry, board_id) do
	    [{pid, _val}] ->
	      # Select all keys from the Job.BoardRegistry:
	      Registry.keys(__MODULE__.BoardRegistry, pid)
	    [] -> []
	  end
      end
      |> Enum.map(&Ecto.UUID.load!/1)

    if Keyword.get(opts, :with_dispatched_crashed) == true do
      Siplane.Repo.all(from j in Siplane.Job, where: (j.id in ^ecto_job_ids) or (is_nil(j.completion_code) and j.dispatched == true))
    else
      Siplane.Repo.all(from j in Siplane.Job, where: j.id in ^ecto_job_ids)
    end
  end

  # TODO: provide an option to include jobs whose end time has passed and thus will never be scheduled?
  def completed_jobs(opts) do
    query = from j in Siplane.Job,
      where: not is_nil(j.completion_code),
      order_by: [desc: j.start]

    query =
      case Keyword.get(opts, :board_id) do
	nil -> query
	board_id ->
          Siplane.Board.validate_board_id(board_id)
	  ecto_board_id = Ecto.UUID.load! board_id
	  where(query, board_id: ^ecto_board_id)
      end

    query =
      case Keyword.get(opts, :limit) do
	nil -> query
	limit_val -> limit(query, ^limit_val)
      end

    Siplane.Repo.all(query)
  end

  # Whether a job is currently scheduled on a board server:
  def job_active?(job_id) do
    validate_job_id(job_id)
    case Registry.lookup(__MODULE__.BoardRegistry, job_id) do
      [{_pid, _val}] -> true
      [] -> false
    end
  end
  
  def get_job_state(job_id) do
    validate_job_id(job_id)
    case Registry.lookup(__MODULE__.BoardRegistry, job_id) do
      [{pid, _val}] -> GenServer.call(pid, {:get_job_state, job_id})
      # TODO!
      [] -> {:error, :job_not_active_todo_this_should_look_into_the_db}
    end
  end

  def instant_job(board_id, environment_id, label \\ nil, creator \\ nil) do
    Siplane.Board.validate_board_id(board_id)
    # TODO: validate environment ID

    creator =
      if !is_nil(creator) do
        Ecto.UUID.load!(creator)
      else
        nil
      end

    IO.inspect(
      %Siplane.Job{
	start: DateTime.truncate(DateTime.utc_now(), :second),
	dispatched: false,
	board_id: Ecto.UUID.load!(board_id),
	environment_id: Ecto.UUID.load!(environment_id),
	label: label,
        creator_id: creator,
      }
    )

    job = Siplane.Repo.insert!(
      %Siplane.Job{
	start: DateTime.truncate(DateTime.utc_now(), :second),
	dispatched: false,
	board_id: Ecto.UUID.load!(board_id),
	environment_id: Ecto.UUID.load!(environment_id),
	label: label,
        creator_id: creator,
      }
    )

    Siplane.Board.jobs_updated(board_id)

    {:ok, job}
  end

  def terminate_job(job_id) do
    validate_job_id(job_id)
    case Registry.lookup(__MODULE__.BoardRegistry, job_id) do
      [{pid, _val}] -> GenServer.call(pid, {:terminate_job, job_id})
      [] -> {:error, :job_not_active}
    end
  end

  # Subscribe to events related to a board
  def subscribe(job_id) do
    validate_job_id(job_id)
    {:ok, _} = Registry.register(__MODULE__.SubscriberRegistry, job_id, nil)
    :ok
  end

  # Retrieve a given number of log messages related to jobs.
  #
  # When nil == 0, this loads all log messages related to this job.
  def job_log(job_id, limit \\ 50) do
    validate_job_id(job_id)
    ecto_job_id = Ecto.UUID.load! job_id

    query = from jl in Siplane.Job.LogEvent,
      join: l in assoc(jl, :log_event),
      where: jl.job_id == ^ecto_job_id,
      order_by: [desc: l.inserted_at]

    query =
      if !is_nil(limit) do
	limit(query, ^limit)
      else
	query
      end

    Siplane.Repo.all(query)
    |> Siplane.Repo.preload(:log_event)
    |> Enum.map(fn job_log_event -> job_log_event.log_event end)
  end

  # Called with the JSON body payload for /api/runner/v0/job/:id/state:
  #
  # TODO: validate payload!
  def update_job_state(job_id, state) do
    validate_job_id(job_id)
    case Registry.lookup(__MODULE__.BoardRegistry, job_id) do
      [{pid, _val}] -> GenServer.call(pid, {:update_job_state, job_id, state})
      [] -> {:error, :job_not_found}
    end
  end

  def put_console_log(job_id, _offset, _next, log) do
    validate_job_id(job_id)
    Registry.dispatch(__MODULE__.SubscriberRegistry, job_id, fn subscribers ->
      for {pid, _} <- subscribers, do: send(pid, {:job_event, job_id, :console_log_event, log})
    end)
  end

  # ----- Supervisor Implementation --------------------------------------------

  @impl true
  def init(_init_arg) do
    children = [
      # Registry of board-servers assigned to jobs
      {Registry, keys: :unique, name: __MODULE__.BoardRegistry},
      # Registry of subscribers to job-events
      {Registry, keys: :duplicate, name: __MODULE__.SubscriberRegistry},
    ]

    Supervisor.init(children, strategy: :one_for_one)
  end
end
