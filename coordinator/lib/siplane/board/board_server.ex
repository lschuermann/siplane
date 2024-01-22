defmodule Siplane.Board.Server.JobState do
  defstruct [
    coord_state: {:wait_for_start, DateTime.utc_now()},
    runner_state: nil,
    # Log of individual lines, reversed (newest to oldest)
    runner_log: [],
  ]
end


defmodule Siplane.Board.Server do
  use GenServer
  import Ecto.Query

  defstruct [
    :board_id,
    :runner_pid,
    # Maximum number of simultaneous jobs. TODO: determine where to get this
    # from. The runner or the database?
    :max_jobs,
    :active_jobs,
    :runner_supervisor_timer,
  ]

  # This module solely contains the GenServer implementation. The public-facing
  # API is provided by the Siplane.Board module (which manages these servers,
  # plus subscribers to board-related events).

  defp arm_runner_supervisor_timer(state) do
    # We want to re-arm the timer if it has expired. For now, let's just
    # periodically send a message. This can be optimized to only send a message
    # when there's active jobs. Check if a timer exists, and if it doesn't, set
    # one. This _should_ be impossible to race, given that this code is
    # exclusive and single-threaded.
    if is_nil(state.runner_supervisor_timer) || !Process.read_timer(state.runner_supervisor_timer) do
      %{ state | runner_supervisor_timer: Process.send_after(self(), :runner_supervisor_timer, 5000) }
    else
      state
    end
  end

  defp complete_jobs(state, job_ids, completion_code) do
    # Remove the job_ids from the active_jobs map. This is more efficient than
    # using Map.filter, as that would have to walk the job_ids list for every
    # active_job.
    active_jobs = List.foldl(
      job_ids,
      state.active_jobs,
      fn job_id, active_jobs ->
	{_, active_jobs} = Map.pop!(active_jobs, job_id)
	active_jobs
      end
    )

    # Set the completion code for all of these jobs in the DB:
    ecto_job_ids = job_ids |> Enum.map(&Ecto.UUID.load!/1)
    Siplane.Repo.update_all(
      from(j in Siplane.Job, where: j.id in ^ecto_job_ids),
      set: [completion_code: completion_code, updated_at: DateTime.utc_now()]
    )

    # Unregister this board server for those jobs
    for job_id <- job_ids do
      :ok = Registry.unregister(Siplane.Job.BoardRegistry, job_id)
    end

    # Return the updated state:
    %{ state | active_jobs: active_jobs }
  end

  # Triggered periodically, through a timer (re)armed by
  # arm_runner_supervisor_timer:
  defp supervise_runners(state) do
    # Fold over active_jobs and state
    {jobs_changed, state} =
      Map.to_list(state.active_jobs)
      |> List.foldl(
	{false, state},
	fn {job_id, job}, {jobs_changed, state} ->
	  case job.coord_state do
	    {:wait_for_start, t} ->
	      if :lt = DateTime.compare(DateTime.add(t, 5, :second), DateTime.utc_now()) do
		{true, complete_jobs(state, [job_id], :runner_timeout)}
	      else
		{jobs_changed, state}
	      end
	    _ ->
	      {jobs_changed, state}
	  end
	end
     )

    if jobs_changed do
      notify_board_update(state, :jobs)
    else
      state
    end
  end

  defp notify_board_update(state, what) do
    Registry.dispatch(Siplane.Board.SubscriberRegistry, state.board_id, fn subscribers ->
      for {pid, _} <- subscribers, do: send(pid, {:board_event, state.board_id, :update, what})
    end)
    state
  end

  # This assumes that we have an established runner connection:
  defp schedule_jobs(state) do
    if map_size(state.active_jobs) < state.max_jobs do
      # TODO: limit to jobs whose start time is reasonably close to the current time...
      case Siplane.Job.pending_jobs(board_id: state.board_id, limit: 1) do
	[] -> state
	[job] ->
	  job_id = UUID.string_to_binary!(job.id)

	  # Register as the board server for this job:
	  {:ok, _} = Registry.register(Siplane.Job.BoardRegistry, job_id, nil)

	  # We've "accepted" this job, so mark it as dispatched in the DB. This
	  # will ensure that we can pick it back up in the future, should this
	  # process crash:
	  Siplane.Repo.update!(Siplane.Job.changeset(job, %{ dispatched: true }))

          # Query the database for the creator's SSH keys
          

	  # Tell the runner:
	  send(
	    state.runner_pid, {
	      :board_event,
	      state.board_id,
	      :runner_msg,
	      %{
		type: :start_job,
		job_id: UUID.binary_to_string!(job_id),
		environment_id: job.environment_id,
		ssh_keys: [
                  "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQC9XgOJNVpO00+p04GwoL0viFco4p24PrsPzvsCOiAETJ6ttYiOFjYrpjQvNLsDse0PcW+duhtjyyp+Y7JZaOTeG6FX1Y8j6gDCUML58f4NlKtnWzfMftYkVO8QeYzdjJhG3J2zBXuqMmen7elVgZouvM/H47X620q7BssTTS1YIVnXxNn5jip8UbSJ1073MnUuTjSGrmS9yyLQx4Ka9/u6zzPDASw6OWXyzoXkWVpU5VzAqYk8Ob9C+lFQw5LMERsQBADoNB+kg/m+OoKS7XXu9+WFdTKsqhy7/c6hijRPaNsP/JRfsxEyjF+Y8LyokU2OXq0MWM0xZrt8O3/DsXAKqyIrGCVPQX+eeXvqP8LusFm2CDe8zoUeyLXvBdZX6Xxyy00OHHQRsuIbBYCJOUMbmDwIcEaC4DELjcNFZXJQYhj2hvB2sQvTMjnS58FDkD+IgVTTlTTzkEiDwbIV6lHbSqzA6AvZtZXd1+rQ4wFIT0clQhAGpBWKN+8Cotl51yC+P+nbU9xlCXM17Q3Pev2sjZFx1VU3oa5SEzEv74aWhUDCaATun/ebi/1Scm3ekOsiPVHuUBm9a2GUMYIuAOTL6W9AM2zZJ8xWkJw5Rj1eQnIDwD9izCRR1gJXlkTYwLn6ZnLawX6FS6Vvj5NV8mjjU4xlhnwaWF0b/Jmm3E966w==",
                  "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDGtanFKH41B3/4piGmhl82gh0AsGACkDV85vUcWHJhrY8AkrVSphnrpL62gY7R6/Rx3AZ+6A+GYoICJa/V4qWUkKryOavZ9xe164JeFtFFN/CrDsIWCLdMyvQ5JO0zc5QmUCRX/UfRzjRzf+y6QdnIoNw5tV2E0zQj9g68I7uNEuuYNmQLg2EyoOE0qtxFTMBEcRzocvVx2QoErDOWD17fDhjc1kR7D7lMVI8jIqRFEQQO3ke6YTXkJAVtjjELqcnSERq58qDKZm9HuYtiUe8cgiBe1UdD8lHTwAGbEE1mSV0IsAOxEoo1BkjBhrYqUxPwfObZwt6TkTF/tbuZkU+m4RU09j2FqliXe66cpDeyAs/C88QObsrqCHb3JJa1GtAp8JSeiELadYYKgiVJibVpb8mBksRxeCruk1Jr9DeAmKVeF/tTect9YALD8qTQNnsyBh0r7HMxyBEze5O6RAV2rnjWHxYIj9oBj8V/q+cykEUXfc4M8rHn/dt72h/LcL3jaYvQwecJjOCW2xW2/sGWkhJ7ittPV7aeOcL9TqG6mggukBjgM/TtPFxsjduxNZ2YSUgVHdmaRsfyDDMfXtdjfCdRcCxslRwQTgl8X2Kb5M0CsQSmWas7vb6JhBLyDPYWWJyxSzb98P3flq9qZjQlVaAiP6asYLDSoKMDYoY7Rw==",
                  "ecdsa-sha2-nistp384 AAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAAAIbmlzdHAzODQAAABhBLRUBvcGp1IPw+c1dBZhe5tpnrd3SwmAPU5NPz0rlVgKrsY0xF6VPRkxx8RiMicT+1lb/gaAgXpjr2gAWOrykx9ICrWOD46LocF7RmYPclhviRoPrIDQ9lr+tgBXVSRKew==",
                  "ecdsa-sha2-nistp384 AAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAAAIbmlzdHAzODQAAABhBAAEifRTFU66SfUXr7eSsBQW1/znzR05dSDCy9eA8eJIbE4kIjqR1hFTKxI+dc2R6jGQK/DxPzLmg/aIRzH6fankH6gWNjpczP0MC1jgSRCqLTQe+TpyVnQ1f/1fPYCSOQ==",
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFjLuM7C6emv+xAFYnXA+mSUKmMEmKDM0A6WzZ0HliFb",
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIKu/2kGWagUgZwwuvjqlStd+iKByxEH+YJQ0tauFp/j",
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB0UMDPmdytTJ2J1y/FsvReZRFTl7WA2HB0GVMWyUP5R",
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICCReHVoVeXfqK8bbIcotwhUBrt7u2T6IQNKMysmJF42",
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGOqwA8XGKxx0RTSmLGVtpWyxkf49B1j+uqNruBRTwem",
                ],
		ssh_rendezvous_servers: [%{
		  client_id: UUID.binary_to_string!(job_id),
		  server_base_url: "https://sns29.cs.princeton.edu/ssh-rendezvous",
		  auth_token: "helloworld",
		}],
	      }
	    }
	  )

	  # Let the subscribers for this job know:
	  # TODO

	  # Update state, (re)set and recursively try to schedule another
	  # job. This is tail-recursive.
	  %{ state | active_jobs: Map.put(state.active_jobs, job_id, %__MODULE__.JobState{}) }
	  |> arm_runner_supervisor_timer
	  |> schedule_jobs
      end
    else
      state
    end
  end

  # ----- GenServer Implementation ---------------------------------------------

  @impl true
  def init(board_id) do
    # First check whether this board actually exists in the
    # database. If it does not, we shouldn't attempt to start a server
    # for it.
    case Siplane.Repo.exists?(from b in Siplane.Board, where: b.id == ^Ecto.UUID.load!(board_id)) do
      false ->
	{:error, :board_id_invalid}

      true ->
	# Register this server before returning. If we race with another server
	# starting for the same board, exit immediately without returning an
	# error (by returning :ignore).
	case Registry.register(__MODULE__.Registry, board_id, nil) do
	  {:error, {:already_registered, _}} ->
	    # We raced, ignore this start request:
	    :ignore

	  {:ok, _} ->
	    # We started & are successfully registered.
	    state = %__MODULE__{
    	      board_id: board_id,
	      runner_pid: nil,
	      max_jobs: 1,
	      active_jobs: %{},
	      runner_supervisor_timer: nil,
            }

	    # Tell all subscribers about the news!
	    notify_board_update(state, {:server_started, self()})

	    # If we were executing some jobs previously (dispatched == true),
	    # but haven't yet completed them, set their completion_code to
	    # :server_crashed. In the future, this allows us to implement a
	    # crash recovery policy.
	    #
	    # Its safe for us to do this here, as we're guaranteed to be the
	    # only board server, and we have not yet run any pending jobs:
	    from(j in Siplane.Job, where: j.board_id == ^Ecto.UUID.load!(board_id) and j.dispatched == true and is_nil(j.completion_code))
	    |> Siplane.Repo.update_all(set: [completion_code: :server_crashed, updated_at: DateTime.utc_now()])

	    # TODO: at some point we should cleanup old servers, when they have
	    # no event subscribers or other dependencies any more. However, we
	    # need to make sure that this doesn't race with any requests
	    # (i.e. don't shut down in between starting the server and servicing
	    # a request). We may be able to enforce this consistency as long as
	    # all other entities interact with this server through the
	    # public-facing interface of this module.

	    {:ok, state}
	end
    end
  end

  @impl true
  def handle_call(:get_runner_connected, _from, state) do
    {
      :reply,
      !is_nil(state.runner_pid) && Process.alive?(state.runner_pid),
      state,
    }
  end

  @impl true
  def handle_call({:connect_runner, conn_info}, {runner_pid, _}, state) do
    # We want to log that the runner disconnected (and update our
    # state), thus monitor the runner connection process. This will
    # still deliver a message even if the process is already dead:
    Process.monitor(runner_pid)

    Siplane.Log.info(
      %Siplane.Log{
	severity: Siplane.Log.severity_info,
	event_type: "board_runner_connected",
	event_version: "0.1",
	message: "Runner has established connection for board ${board_id}.",
	data: %{
	  board_id: UUID.binary_to_string!(state.board_id),
	  conn_info: conn_info,
	},
	log_event_boards: [
	  %Siplane.Board.LogEvent{
	    board_id: Ecto.UUID.load!(state.board_id),
	    public: true,
	    owner_visible: true,
          }
	],
      }
    )

    state =
      %{ state | runner_pid: runner_pid }
      |> schedule_jobs
      |> notify_board_update(:runner_connected)

    {
      :reply,
      {
	:ok,
	# TODO: craft an appropriate last will and testament message
	"oops I'm dead!",
      },
      state,
    }
  end

  @impl true
  def handle_call({:get_job_state, job_id}, _from, state) do
    case Map.get(state.active_jobs, job_id) do
      nil ->
        {:reply, {:error, :job_not_active}, state}
      
      job_state ->
        {:reply, {:ok, job_state}, state}
    end
  end

  @impl true
  def handle_call({:update_job_state, job_id, new_runner_state}, _from, state) do
    case Map.get(state.active_jobs, job_id) do
      nil ->
	{:reply, {:error, :job_not_active}, state}

      job_state ->
	act =
	  case {job_state.coord_state, new_runner_state.state} do
	    {{:wait_for_start, _}, :starting} ->
	      {:ok, {:starting, DateTime.utc_now()}}
	    {{:wait_for_start, _}, :ready} ->
	      {:ok, :ready}
	    {{:starting, ts}, :starting} ->
	      {:ok, {:starting, ts}}
	    {{:starting, _}, :ready} ->
	      {:ok, :ready}
	    {:ready, :ready} ->
	      {:ok, :ready}
	    {_, :stopping} ->
	      {:ok, {:stopping, DateTime.utc_now()}}
	    {_, :finished} ->
	      {:remove, :finished}
	    {_, :failed} ->
	      {:remove, :failed}
	  end

	notify_board_update(state, :jobs)

	state =
	  case act do
	    {:ok, new_coord_state} ->
	      new_jobs = Map.put(state.active_jobs, job_id, %{ job_state | coord_state: new_coord_state, runner_state: new_runner_state })
	      %{ state | active_jobs: new_jobs }
	    {:remove, completion_code} ->
	      complete_jobs(state, [job_id], completion_code)
	  end

	state = schedule_jobs(state)

	{:reply, :ok, state}
    end
  end

  @impl true
  def handle_call({:terminate_job, job_id}, _from, state) do
    # Tell the runner:
    send(
      state.runner_pid, {
	:board_event,
	state.board_id,
	:runner_msg,
	%{
	  type: :stop_job,
	  job_id: UUID.binary_to_string!(job_id),
	}
      }
    )

    {:reply, :ok, state}
  end

  @impl true
  def handle_cast(:schedule_jobs, state) do
    {:noreply, schedule_jobs(state)}
  end

  @impl true
  def handle_info({:DOWN, _ref, :process, runner_pid, _type}, %{ runner_pid: runner_pid } = state) do
    # Mark all active jobs as failed and remove them from the board registry and
    # state. We may want to implement a retry-policy later:
    state =
      %{ state | runner_pid: nil }
      |> complete_jobs(Map.keys(state.active_jobs), :failed)

    # TODO: different log message when there were active jobs
    Siplane.Log.info(
      %Siplane.Log{
	severity: Siplane.Log.severity_info,
	event_type: "board_runner_disconnected",
	event_version: "0.1",
	message: "The connection to the runner for board ${board_id} has been lost.",
	data: %{
	  board_id: UUID.binary_to_string!(state.board_id),
	},
	log_event_boards: [
	  %Siplane.Board.LogEvent{
	    board_id: Ecto.UUID.load!(state.board_id),
	    public: true,
	    owner_visible: true,
          }
	],
	# TODO: attach this to all the jobs we've just marked as failed!
      }
    )

    notify_board_update(state, :runner_disconnected)

    {
      :noreply,
      %{
	state | runner_pid: nil
      },
    }
  end

  @impl true
  def handle_info(:runner_supervisor_timer, state) do
    # Rearm the timer, if necessary, after processing the supervision routine:
    {:noreply, arm_runner_supervisor_timer(supervise_runners(state)) }
  end
end
