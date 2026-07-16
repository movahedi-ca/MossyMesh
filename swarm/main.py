from crewai import Crew, Process
from agents import create_agents
from tasks import create_tasks

def run_mossymesh_swarm():
    print("Initializing the 16-Agent MossyMesh Swarm...")
    
    # 1. Instantiate the 16 agents
    agents_dict = create_agents()
    
    # 2. Define the tasks for the current execution phase
    tasks = create_tasks(agents_dict)
    
    # 3. Form the Crew
    # We pass all agents to the crew. CrewAI will orchestrate them.
    mossymesh_crew = Crew(
        agents=list(agents_dict.values()),
        tasks=tasks,
        process=Process.hierarchical,
        manager_llm=agents_dict['lead_architect'].llm, # The Lead Architect acts as the manager
        verbose=True
    )
    
    # 4. Kickoff the execution
    print("Kicking off Phase 1 Autonomous Build...")
    result = mossymesh_crew.kickoff()
    
    print("\n\n######################")
    print("## EXECUTION RESULT ##")
    print("######################\n")
    print(result)

if __name__ == "__main__":
    run_mossymesh_swarm()
