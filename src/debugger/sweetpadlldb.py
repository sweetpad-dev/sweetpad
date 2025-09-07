import lldb


def first_utility(debugger, command, result, internal_dict):
    print("I am the first utility", file=result)

def second_utility(debugger, command, result, internal_dict):
    print("I am the second utility", file=result)


# And the initialization code to add your commands
def __lldb_init_module(debugger, internal_dict):
    debugger.HandleCommand('command container add -h "A container for my utilities" sweetpad')
    debugger.HandleCommand('command script add -f sweetpad.first_utility -h "My first utility" sweetpad first')
    debugger.HandleCommand('command script add -f sweetpad.second_utility -h "My second utility" sweetpad second')
    print('The "sweetpad" python command has been installed and its subcommands are ready for use.')